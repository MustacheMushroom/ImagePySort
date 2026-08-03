//! Durable, incremental duplicate-scan cache.
//!
//! The active scan is written beside the last completed generation. Only after
//! traversal and hashing finish successfully does it replace the old generation.
//! Cancellation, process termination, and routine scan errors therefore leave the
//! previous saved results available.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};
use walkdir::WalkDir;

use crate::{DuplicateGroups, ScanProgress, absolute_path, file_hash_with_cancel, media_kind};

const CACHE_FILE_NAME: &str = "scan-cache.sqlite3";
const SCHEMA_VERSION: i64 = 1;
const INSERT_COMMIT_INTERVAL: usize = 5_000;
const HASH_BATCH_SIZE: usize = 256;

/// Whether a scan may reuse hashes from the last completed generation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScanMode {
    #[default]
    Incremental,
    Full,
}

/// A completed scan restored from, or newly committed to, the local cache.
#[derive(Debug, Clone)]
pub struct CachedScan {
    pub roots: Vec<PathBuf>,
    pub groups: DuplicateGroups,
    pub progress: ScanProgress,
    pub completed_at_unix_seconds: i64,
}

/// Final state of a cancellable cached scan.
#[derive(Debug, Clone)]
pub enum CachedScanOutcome {
    Completed(CachedScan),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileFingerprint {
    size: i64,
    modified_ns: i64,
    created_ns: i64,
}

struct CandidateFile {
    path: Vec<u8>,
    hash: Option<String>,
    fingerprint: FileFingerprint,
}

/// Location of MediaSift's machine-local scan cache.
pub fn cache_database_path() -> io::Result<PathBuf> {
    ProjectDirs::from("com", "MustacheMushroom", "MediaSift")
        .map(|directories| directories.data_local_dir().join(CACHE_FILE_NAME))
        .ok_or_else(|| io::Error::other("Windows did not provide a Local AppData directory."))
}

/// Load the last successfully committed scan, if one exists.
pub fn load_cached_scan() -> io::Result<Option<CachedScan>> {
    load_cached_scan_at(&cache_database_path()?)
}

/// Delete saved paths, file metadata, and hashes. Media files are never touched.
pub fn forget_cached_scan() -> io::Result<()> {
    let path = cache_database_path()?;
    forget_cached_scan_at(&path)
}

fn forget_cached_scan_at(path: &Path) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut cache_file = path.as_os_str().to_os_string();
        cache_file.push(suffix);
        match fs::remove_file(PathBuf::from(cache_file)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Scan media with a disk-backed index and atomically replace the previous cache.
pub fn find_exact_duplicates_with_cache_and_cancel<F, C>(
    roots: &[PathBuf],
    mode: ScanMode,
    report_progress: F,
    should_cancel: C,
) -> io::Result<CachedScanOutcome>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    scan_at(
        &cache_database_path()?,
        roots,
        mode,
        report_progress,
        should_cancel,
    )
}

fn scan_at<F, C>(
    cache_path: &Path,
    roots: &[PathBuf],
    mode: ScanMode,
    mut report_progress: F,
    mut should_cancel: C,
) -> io::Result<CachedScanOutcome>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    if roots.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Select at least one scan location.",
        ));
    }

    let roots = roots
        .iter()
        .filter(|root| root.is_dir())
        .map(|root| absolute_path(root))
        .collect::<io::Result<Vec<_>>>()?;
    if roots.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "None of the selected scan locations are currently available.",
        ));
    }

    let connection = open_cache(cache_path)?;
    let previous_scan_id = latest_scan_id(&connection)?;
    connection
        .execute(
            "INSERT INTO scans (status, started_at, mode) VALUES ('building', ?1, ?2)",
            params![unix_seconds_now(), mode_label(mode)],
        )
        .map_err(sql_error)?;
    let scan_id = connection.last_insert_rowid();
    for (ordinal, root) in roots.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO scan_roots (scan_id, ordinal, path) VALUES (?1, ?2, ?3)",
                params![scan_id, ordinal as i64, path_to_blob(root)],
            )
            .map_err(sql_error)?;
    }

    let result = build_generation(
        &connection,
        scan_id,
        previous_scan_id,
        &roots,
        mode,
        &mut report_progress,
        &mut should_cancel,
    );
    match result {
        Ok(Some(scan)) => Ok(CachedScanOutcome::Completed(scan)),
        Ok(None) => {
            discard_generation(&connection, scan_id);
            Ok(CachedScanOutcome::Cancelled)
        }
        Err(error) => {
            discard_generation(&connection, scan_id);
            Err(error)
        }
    }
}

fn build_generation<F, C>(
    connection: &Connection,
    scan_id: i64,
    previous_scan_id: Option<i64>,
    roots: &[PathBuf],
    mode: ScanMode,
    report_progress: &mut F,
    should_cancel: &mut C,
) -> io::Result<Option<CachedScan>>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    let mut progress = ScanProgress::default();
    let mut pending_inserts = 0_usize;
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(sql_error)?;

    for root in roots {
        if should_cancel() {
            report_progress(progress);
            return Ok(None);
        }
        progress.roots_started += 1;
        for entry in WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
        {
            if should_cancel() {
                report_progress(progress);
                return Ok(None);
            }
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_type().is_dir() {
                progress.directories_visited += 1;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            progress.files_visited += 1;
            let path = entry.into_path();
            if media_kind(&path).is_none() {
                maybe_report_traversal(&progress, report_progress);
                continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let Some(fingerprint) = fingerprint(&metadata) else {
                continue;
            };
            let path_blob = path_to_blob(&path);
            let reused_hash = if mode == ScanMode::Incremental {
                previous_hash(connection, previous_scan_id, &path_blob, fingerprint)?
            } else {
                None
            };
            let inserted = connection
                .execute(
                    "INSERT OR IGNORE INTO files
                     (scan_id, path, size, modified_ns, created_ns, hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        scan_id,
                        path_blob,
                        fingerprint.size,
                        fingerprint.modified_ns,
                        fingerprint.created_ns,
                        reused_hash
                    ],
                )
                .map_err(sql_error)?;
            if inserted == 1 {
                progress.media_files_found += 1;
                if reused_hash.is_some() {
                    progress.hashes_reused += 1;
                }
                pending_inserts += 1;
            }
            if pending_inserts >= INSERT_COMMIT_INTERVAL {
                connection
                    .execute_batch("COMMIT; BEGIN IMMEDIATE")
                    .map_err(sql_error)?;
                pending_inserts = 0;
            }
            maybe_report_traversal(&progress, report_progress);
        }
        report_progress(progress.clone());
    }
    connection.execute_batch("COMMIT").map_err(sql_error)?;

    let candidate_sizes = candidate_sizes(connection, scan_id)?;
    for size in candidate_sizes {
        let mut after_path: Option<Vec<u8>> = None;
        loop {
            if should_cancel() {
                report_progress(progress);
                return Ok(None);
            }
            let batch = candidate_batch(connection, scan_id, size, after_path.as_deref())?;
            if batch.is_empty() {
                break;
            }
            after_path = batch.last().map(|file| file.path.clone());
            for file in batch {
                if should_cancel() {
                    report_progress(progress);
                    return Ok(None);
                }
                let path = path_from_blob(&file.path)?;
                let current_fingerprint = fs::metadata(&path)
                    .ok()
                    .and_then(|metadata| fingerprint(&metadata));
                if current_fingerprint != Some(file.fingerprint) {
                    delete_indexed_file(connection, scan_id, &file.path)?;
                    continue;
                }
                if file.hash.is_some() {
                    continue;
                }
                match file_hash_with_cancel(&path, should_cancel) {
                    Ok(Some(hash)) => {
                        let fingerprint_after_hash = fs::metadata(&path)
                            .ok()
                            .and_then(|metadata| fingerprint(&metadata));
                        if fingerprint_after_hash == Some(file.fingerprint) {
                            connection
                                .execute(
                                    "UPDATE files SET hash = ?1 WHERE scan_id = ?2 AND path = ?3",
                                    params![hash, scan_id, file.path],
                                )
                                .map_err(sql_error)?;
                        } else {
                            delete_indexed_file(connection, scan_id, &file.path)?;
                        }
                    }
                    Ok(None) => {
                        report_progress(progress);
                        return Ok(None);
                    }
                    Err(_) => {
                        delete_indexed_file(connection, scan_id, &file.path)?;
                    }
                }
                progress.files_hashed += 1;
                if progress.files_hashed.is_multiple_of(25) {
                    report_progress(progress.clone());
                }
            }
        }
    }

    let groups = duplicate_groups(connection, scan_id)?;
    progress.duplicate_groups = groups.len();
    report_progress(progress.clone());
    let completed_at = unix_seconds_now();
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(sql_error)?;
    connection
        .execute(
            "UPDATE scans SET status = 'complete', completed_at = ?1,
             directories_visited = ?2, files_visited = ?3, media_files_found = ?4,
             files_hashed = ?5, hashes_reused = ?6, duplicate_groups = ?7
             WHERE id = ?8",
            params![
                completed_at,
                usize_to_i64(progress.directories_visited),
                usize_to_i64(progress.files_visited),
                usize_to_i64(progress.media_files_found),
                usize_to_i64(progress.files_hashed),
                usize_to_i64(progress.hashes_reused),
                usize_to_i64(progress.duplicate_groups),
                scan_id,
            ],
        )
        .map_err(sql_error)?;
    connection
        .execute("DELETE FROM scans WHERE id <> ?1", [scan_id])
        .map_err(sql_error)?;
    connection.execute_batch("COMMIT").map_err(sql_error)?;

    Ok(Some(CachedScan {
        roots: roots.to_vec(),
        groups,
        progress,
        completed_at_unix_seconds: completed_at,
    }))
}

fn open_cache(path: &Path) -> io::Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path).map_err(sql_error)?;
    let schema_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(sql_error)?;
    if schema_version > SCHEMA_VERSION {
        return Err(io::Error::other(format!(
            "The saved scan uses schema version {schema_version}, but this MediaSift build supports version {SCHEMA_VERSION}."
        )));
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS scans (
                 id INTEGER PRIMARY KEY,
                 status TEXT NOT NULL,
                 started_at INTEGER NOT NULL,
                 completed_at INTEGER,
                 mode TEXT NOT NULL,
                 directories_visited INTEGER NOT NULL DEFAULT 0,
                 files_visited INTEGER NOT NULL DEFAULT 0,
                 media_files_found INTEGER NOT NULL DEFAULT 0,
                 files_hashed INTEGER NOT NULL DEFAULT 0,
                 hashes_reused INTEGER NOT NULL DEFAULT 0,
                 duplicate_groups INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS scan_roots (
                 scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                 ordinal INTEGER NOT NULL,
                 path BLOB NOT NULL,
                 PRIMARY KEY (scan_id, ordinal)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS files (
                 scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                 path BLOB NOT NULL,
                 size INTEGER NOT NULL,
                 modified_ns INTEGER NOT NULL,
                 created_ns INTEGER NOT NULL,
                 hash TEXT,
                 PRIMARY KEY (scan_id, path)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS files_by_scan_size ON files(scan_id, size);
             CREATE INDEX IF NOT EXISTS files_by_scan_hash ON files(scan_id, hash);",
        )
        .map_err(sql_error)?;
    if schema_version == 0 {
        connection
            .execute_batch("PRAGMA user_version = 1")
            .map_err(sql_error)?;
    }
    connection
        .execute("DELETE FROM scans WHERE status <> 'complete'", [])
        .map_err(sql_error)?;
    Ok(connection)
}

fn latest_scan_id(connection: &Connection) -> io::Result<Option<i64>> {
    connection
        .query_row(
            "SELECT id FROM scans WHERE status = 'complete' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)
}

fn load_cached_scan_at(path: &Path) -> io::Result<Option<CachedScan>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = open_cache(path)?;
    let Some(scan_id) = latest_scan_id(&connection)? else {
        return Ok(None);
    };
    let (completed_at, progress) = connection
        .query_row(
            "SELECT completed_at, directories_visited, files_visited, media_files_found,
                    files_hashed, hashes_reused, duplicate_groups
             FROM scans WHERE id = ?1",
            [scan_id],
            |row| {
                Ok((
                    row.get(0)?,
                    ScanProgress {
                        directories_visited: i64_to_usize(row.get(1)?),
                        files_visited: i64_to_usize(row.get(2)?),
                        media_files_found: i64_to_usize(row.get(3)?),
                        files_hashed: i64_to_usize(row.get(4)?),
                        hashes_reused: i64_to_usize(row.get(5)?),
                        duplicate_groups: i64_to_usize(row.get(6)?),
                        ..Default::default()
                    },
                ))
            },
        )
        .map_err(sql_error)?;
    let roots = {
        let mut statement = connection
            .prepare("SELECT path FROM scan_roots WHERE scan_id = ?1 ORDER BY ordinal")
            .map_err(sql_error)?;
        statement
            .query_map([scan_id], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sql_error)?
            .map(|value| {
                value
                    .map_err(sql_error)
                    .and_then(|blob| path_from_blob(&blob))
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    let groups = duplicate_groups(&connection, scan_id)?;
    Ok(Some(CachedScan {
        roots,
        groups,
        progress,
        completed_at_unix_seconds: completed_at,
    }))
}

fn previous_hash(
    connection: &Connection,
    previous_scan_id: Option<i64>,
    path: &[u8],
    current: FileFingerprint,
) -> io::Result<Option<String>> {
    let Some(previous_scan_id) = previous_scan_id else {
        return Ok(None);
    };
    let previous = connection
        .query_row(
            "SELECT size, modified_ns, created_ns, hash
             FROM files WHERE scan_id = ?1 AND path = ?2",
            params![previous_scan_id, path],
            |row| {
                Ok((
                    FileFingerprint {
                        size: row.get(0)?,
                        modified_ns: row.get(1)?,
                        created_ns: row.get(2)?,
                    },
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    Ok(previous.and_then(|(fingerprint, hash)| (fingerprint == current).then_some(hash).flatten()))
}

fn candidate_sizes(connection: &Connection, scan_id: i64) -> io::Result<Vec<i64>> {
    let mut statement = connection
        .prepare("SELECT size FROM files WHERE scan_id = ?1 GROUP BY size HAVING COUNT(*) > 1")
        .map_err(sql_error)?;
    statement
        .query_map([scan_id], |row| row.get(0))
        .map_err(sql_error)?
        .map(|value| value.map_err(sql_error))
        .collect()
}

fn candidate_batch(
    connection: &Connection,
    scan_id: i64,
    size: i64,
    after_path: Option<&[u8]>,
) -> io::Result<Vec<CandidateFile>> {
    let mut statement = connection
        .prepare(
            "SELECT path, hash, size, modified_ns, created_ns FROM files
             WHERE scan_id = ?1 AND size = ?2 AND (?3 IS NULL OR path > ?3)
             ORDER BY path LIMIT ?4",
        )
        .map_err(sql_error)?;
    statement
        .query_map(
            params![scan_id, size, after_path, HASH_BATCH_SIZE as i64],
            |row| {
                Ok(CandidateFile {
                    path: row.get(0)?,
                    hash: row.get(1)?,
                    fingerprint: FileFingerprint {
                        size: row.get(2)?,
                        modified_ns: row.get(3)?,
                        created_ns: row.get(4)?,
                    },
                })
            },
        )
        .map_err(sql_error)?
        .map(|value| value.map_err(sql_error))
        .collect()
}

fn delete_indexed_file(connection: &Connection, scan_id: i64, path: &[u8]) -> io::Result<()> {
    connection
        .execute(
            "DELETE FROM files WHERE scan_id = ?1 AND path = ?2",
            params![scan_id, path],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn duplicate_groups(connection: &Connection, scan_id: i64) -> io::Result<DuplicateGroups> {
    let hashes = {
        let mut statement = connection
            .prepare(
                "SELECT hash FROM files
                 WHERE scan_id = ?1 AND hash IS NOT NULL
                 GROUP BY hash HAVING COUNT(*) > 1 ORDER BY hash",
            )
            .map_err(sql_error)?;
        statement
            .query_map([scan_id], |row| row.get::<_, String>(0))
            .map_err(sql_error)?
            .map(|value| value.map_err(sql_error))
            .collect::<io::Result<Vec<_>>>()?
    };
    let mut groups = BTreeMap::new();
    let mut statement = connection
        .prepare("SELECT path FROM files WHERE scan_id = ?1 AND hash = ?2 ORDER BY path")
        .map_err(sql_error)?;
    for hash in hashes {
        let paths = statement
            .query_map(params![scan_id, hash], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sql_error)?
            .map(|value| {
                value
                    .map_err(sql_error)
                    .and_then(|blob| path_from_blob(&blob))
            })
            .collect::<io::Result<Vec<_>>>()?;
        groups.insert(hash, paths);
    }
    Ok(groups)
}

fn fingerprint(metadata: &fs::Metadata) -> Option<FileFingerprint> {
    Some(FileFingerprint {
        size: i64::try_from(metadata.len()).ok()?,
        modified_ns: system_time_ns(metadata.modified().ok()?),
        created_ns: metadata.created().map(system_time_ns).unwrap_or(0),
    })
}

fn system_time_ns(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_nanos()).unwrap_or(i64::MAX),
    }
}

fn unix_seconds_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn mode_label(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Incremental => "incremental",
        ScanMode::Full => "full",
    }
}

fn maybe_report_traversal<F>(progress: &ScanProgress, report_progress: &mut F)
where
    F: FnMut(ScanProgress),
{
    if progress.files_visited.is_multiple_of(250) {
        report_progress(progress.clone());
    }
}

fn discard_generation(connection: &Connection, scan_id: i64) {
    let _ = connection.execute_batch("ROLLBACK");
    let _ = connection.execute("DELETE FROM scans WHERE id = ?1", [scan_id]);
}

fn sql_error(error: rusqlite::Error) -> io::Error {
    io::Error::other(format!(
        "Could not update the MediaSift scan cache: {error}"
    ))
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or_default()
}

#[cfg(windows)]
fn path_to_blob(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn path_from_blob(blob: &[u8]) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    if !blob.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The saved scan contains an invalid Windows path.",
        ));
    }
    let wide = blob
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(unix)]
fn path_to_blob(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from_blob(blob: &[u8]) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(blob.to_vec())))
}

#[cfg(not(any(windows, unix)))]
fn path_to_blob(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[cfg(not(any(windows, unix)))]
fn path_from_blob(blob: &[u8]) -> io::Result<PathBuf> {
    String::from_utf8(blob.to_vec())
        .map(PathBuf::from)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "media-sift-cache-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn completed(outcome: CachedScanOutcome) -> CachedScan {
        match outcome {
            CachedScanOutcome::Completed(scan) => scan,
            CachedScanOutcome::Cancelled => panic!("scan should complete"),
        }
    }

    #[test]
    fn unchanged_files_reuse_saved_hashes() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        fs::write(root.join("one.jpg"), b"same media bytes").unwrap();
        fs::write(root.join("two.jpg"), b"same media bytes").unwrap();

        let first = completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );
        assert_eq!(first.groups.len(), 1);
        assert_eq!(first.progress.files_hashed, 2);
        assert_eq!(first.progress.hashes_reused, 0);

        let second = completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );
        assert_eq!(second.groups, first.groups);
        assert_eq!(second.progress.files_hashed, 0);
        assert_eq!(second.progress.hashes_reused, 2);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_and_new_files_refresh_only_needed_hashes() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        fs::write(root.join("one.jpg"), b"same media bytes").unwrap();
        fs::write(root.join("two.jpg"), b"same media bytes").unwrap();
        completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );

        fs::write(
            root.join("two.jpg"),
            b"different content with a deliberately unique length",
        )
        .unwrap();
        fs::write(root.join("three.jpg"), b"same media bytes").unwrap();
        let refreshed = completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );

        assert_eq!(refreshed.groups.len(), 1);
        assert_eq!(refreshed.progress.files_hashed, 1);
        assert_eq!(refreshed.progress.hashes_reused, 1);
        assert!(
            refreshed
                .groups
                .values()
                .flatten()
                .any(|path| path.ends_with("three.jpg"))
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_preserves_last_completed_generation() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        fs::write(root.join("one.jpg"), b"same media bytes").unwrap();
        fs::write(root.join("two.jpg"), b"same media bytes").unwrap();
        let first = completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );

        let cancelled = scan_at(
            &cache,
            std::slice::from_ref(&root),
            ScanMode::Full,
            |_| {},
            || true,
        )
        .unwrap();
        assert!(matches!(cancelled, CachedScanOutcome::Cancelled));
        let restored = load_cached_scan_at(&cache).unwrap().unwrap();
        assert_eq!(restored.groups, first.groups);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_rescan_ignores_saved_hashes() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        fs::write(root.join("one.jpg"), b"same media bytes").unwrap();
        fs::write(root.join("two.jpg"), b"same media bytes").unwrap();
        completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Incremental,
                |_| {},
                || false,
            )
            .unwrap(),
        );

        let full = completed(
            scan_at(
                &cache,
                std::slice::from_ref(&root),
                ScanMode::Full,
                |_| {},
                || false,
            )
            .unwrap(),
        );
        assert_eq!(full.progress.files_hashed, 2);
        assert_eq!(full.progress.hashes_reused, 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_cache_and_sidecars_can_always_be_forgotten() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        fs::write(&cache, b"not a sqlite database").unwrap();
        fs::write(root.join("cache.sqlite3-wal"), b"wal").unwrap();
        fs::write(root.join("cache.sqlite3-shm"), b"shm").unwrap();

        forget_cached_scan_at(&cache).unwrap();

        assert!(!cache.exists());
        assert!(!root.join("cache.sqlite3-wal").exists());
        assert!(!root.join("cache.sqlite3-shm").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newer_cache_schema_is_rejected_without_mutation() {
        let root = temp_dir();
        let cache = root.join("cache.sqlite3");
        let connection = Connection::open(&cache).unwrap();
        connection.execute_batch("PRAGMA user_version = 2").unwrap();
        drop(connection);

        let error = load_cached_scan_at(&cache).unwrap_err();
        assert!(error.to_string().contains("schema version 2"));

        fs::remove_dir_all(root).unwrap();
    }
}
