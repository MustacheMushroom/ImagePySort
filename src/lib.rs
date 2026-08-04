//! Core file-system operations and native desktop application for MediaSift.

pub mod desktop;
pub mod scan_cache;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Datelike, Local, NaiveDate};
use exif::{In, Reader, Tag, Value};
use image::{ImageFormat, Rgb, RgbImage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Image extensions supported by both sorting and duplicate scanning.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp"];
/// Common image formats, including modern and camera-raw formats.
pub const IMAGE_MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "avif", "dng",
    "raw", "cr2", "cr3", "nef", "arw", "raf", "rw2", "orf", "pef", "srw",
];
/// Common video formats eligible for exact byte-for-byte duplicate matching.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "mpg", "mpeg", "3gp", "3g2", "flv", "f4v",
    "ogv", "ts", "m2ts", "mts", "vob", "asf", "rm", "rmvb",
];
/// Common audio formats eligible for exact byte-for-byte duplicate matching.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff", "aif", "alac", "ape", "amr",
    "ac3", "dts", "mid", "midi",
];
const HASH_CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveCompression {
    Stored,
    Fast,
    Default,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileActionSummary {
    pub processed: usize,
    pub failures: Vec<String>,
}

/// Result of adding date prefixes to media filenames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameSummary {
    pub renamed: usize,
    pub corrected_prefixes: usize,
    pub capture_dates_used: usize,
    pub filesystem_dates_used: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
}

/// User-selected behavior for adding dates to media filenames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DatePrefixOptions {
    /// For media without an embedded capture date, use the earlier filesystem
    /// timestamp instead of preferring creation time.
    pub use_oldest_filesystem_date: bool,
    /// Replace an existing `YYYY-MM-DD - ` prefix when it disagrees with the
    /// preferred date. Disabled by default to preserve user-authored names.
    pub correct_existing_prefixes: bool,
}

/// A conservative local-restoration prompt for use with an external image model.
pub const RESTORATION_PROMPT: &str = "Colorize and clean up the attached historical photograph with strict 100% preservation of all subjects' exact faces, expressions, and identities.\n\nSTRICT CONSTRAINT: Do NOT alter, re-imagine, morph, or over-smooth any faces, eyes, smiles, noses, lips, or expressions. Keep all facial features 100% faithful to the original photograph.\n\nPerform a conservative restoration:\n- Remove dust, scratches, cracks, fading, scanner glare, and yellowing/sepia tinting.\n- Preserve 100% of original facial contours, eye shapes, nose, lips, hair, and clothing silhouettes.\n- Apply historically accurate, muted colorization with natural skin tones and era-appropriate clothing hues.\n- Maintain original lighting, shadows, and natural film grain character.";

/// Duplicate paths grouped by their SHA-256 file-content hash.
pub type DuplicateGroups = BTreeMap<String, Vec<PathBuf>>;

/// Files from one duplicate group that must still match immediately before a
/// destructive action is allowed to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateActionVerification {
    pub expected_hash: String,
    pub keeper: PathBuf,
    pub selected: Vec<PathBuf>,
}

/// Live, monotonic counters emitted while a media scan is running.
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub roots_started: usize,
    pub directories_visited: usize,
    pub files_visited: usize,
    pub media_files_found: usize,
    pub files_hashed: usize,
    /// File hashes reused after their size and timestamps matched the saved scan.
    pub hashes_reused: usize,
    pub duplicate_groups: usize,
}

/// Final state of a cancellable exact-duplicate scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateScanOutcome {
    Completed(DuplicateGroups),
    Cancelled,
}

/// Return the kind of a supported, known media file. Unknown extensions are excluded.
pub fn media_kind(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_str()?;
    let in_list = |list: &[&str]| {
        list.iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    };
    if in_list(IMAGE_MEDIA_EXTENSIONS) {
        Some(MediaKind::Image)
    } else if in_list(VIDEO_EXTENSIONS) {
        Some(MediaKind::Video)
    } else if in_list(AUDIO_EXTENSIONS) {
        Some(MediaKind::Audio)
    } else {
        None
    }
}

/// Enumerate requested media paths below all `roots`, deduplicating overlapping roots.
pub fn media_paths(roots: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    if roots.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Select at least one scan location.",
        ));
    }
    let mut paths = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let root = absolute_path(root)?;
        paths.extend(
            WalkDir::new(root)
                .follow_links(false)
                .sort_by_file_name()
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file() && media_kind(entry.path()).is_some())
                .map(|entry| entry.into_path()),
        );
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Return supported image paths below `directory`, without following symlinks.
pub fn image_paths(directory: &Path) -> io::Result<Vec<PathBuf>> {
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Directory does not exist: {}", directory.display()),
        ));
    }

    let root = absolute_path(directory)?;
    let mut paths: Vec<_> = WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_image_path(entry.path()))
        .map(|entry| entry.into_path())
        .collect();
    paths.sort();
    Ok(paths)
}

/// Return the SHA-256 hash of a file's raw bytes.
pub fn file_hash(path: &Path) -> io::Result<String> {
    file_hash_with_cancel(path, &mut || false)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "File hashing was cancelled."))
}

/// Re-hash selected duplicates and one retained copy from every affected group.
///
/// Cached size and timestamp metadata make refreshes fast, but are not a safe
/// authorization boundary for deletion. Call this immediately before moving or
/// deleting files so a same-size replacement cannot be treated as a duplicate.
pub fn verify_duplicate_action_groups(groups: &[DuplicateActionVerification]) -> io::Result<()> {
    for group in groups {
        for path in std::iter::once(&group.keeper).chain(&group.selected) {
            let actual_hash = file_hash(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Could not verify {} before changing files: {error}. Refresh the scan and review the group again.",
                        path.display()
                    ),
                )
            })?;
            if actual_hash != group.expected_hash {
                return Err(io::Error::other(format!(
                    "{} changed after the scan and is no longer an exact duplicate. Refresh the scan and review the group again.",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn file_hash_with_cancel<C>(path: &Path, should_cancel: &mut C) -> io::Result<Option<String>>
where
    C: FnMut() -> bool,
{
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; HASH_CHUNK_SIZE];

    loop {
        if should_cancel() {
            return Ok(None);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(Some(format!("{:x}", hasher.finalize())))
}

/// Find byte-for-byte duplicate images below `directory`.
///
/// Files are first grouped by size, so files with unique sizes are never read
/// for hashing. Files that disappear or become unreadable during a scan are
/// skipped, allowing long drive scans to complete.
pub fn find_exact_duplicates(directory: &Path) -> io::Result<DuplicateGroups> {
    find_exact_duplicates_in_roots(&[directory.to_path_buf()])
}

/// Find byte-for-byte duplicate known media files across folders and drives.
pub fn find_exact_duplicates_in_roots(roots: &[PathBuf]) -> io::Result<DuplicateGroups> {
    find_exact_duplicates_with_progress(roots, |_| {})
}

/// Find exact duplicate known media files while reporting scanner progress.
pub fn find_exact_duplicates_with_progress<F>(
    roots: &[PathBuf],
    report_progress: F,
) -> io::Result<DuplicateGroups>
where
    F: FnMut(ScanProgress),
{
    match find_exact_duplicates_with_progress_and_cancel(roots, report_progress, || false)? {
        DuplicateScanOutcome::Completed(groups) => Ok(groups),
        DuplicateScanOutcome::Cancelled => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Duplicate scan was cancelled.",
        )),
    }
}

/// Find exact duplicate media while reporting progress and polling for cancellation.
///
/// Cancellation is cooperative and checked between directory entries and every
/// hash read chunk, so the caller can stop long scans without blocking the UI.
pub fn find_exact_duplicates_with_progress_and_cancel<F, C>(
    roots: &[PathBuf],
    mut report_progress: F,
    mut should_cancel: C,
) -> io::Result<DuplicateScanOutcome>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    require_scan_roots(roots)?;
    let mut progress = ScanProgress::default();
    let mut paths_by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut seen_media_paths = HashSet::new();
    for root in roots {
        if should_cancel() {
            report_progress(progress);
            return Ok(DuplicateScanOutcome::Cancelled);
        }
        if !root.is_dir() {
            continue;
        }
        let root = absolute_path(root)?;
        progress.roots_started += 1;
        for entry in WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
        {
            if should_cancel() {
                report_progress(progress);
                return Ok(DuplicateScanOutcome::Cancelled);
            }
            let Ok(entry) = entry else {
                continue;
            };
            let file_type = entry.file_type();
            let path = entry.into_path();
            if file_type.is_dir() {
                progress.directories_visited += 1;
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            progress.files_visited += 1;
            if media_kind(&path).is_some() {
                if !seen_media_paths.insert(path.clone()) {
                    continue;
                }
                progress.media_files_found += 1;
                if let Ok(metadata) = fs::metadata(&path) {
                    paths_by_size.entry(metadata.len()).or_default().push(path);
                }
            }
            if progress.files_visited % 250 == 0 {
                report_progress(progress.clone());
            }
        }
        report_progress(progress.clone());
    }

    let mut paths_by_hash: DuplicateGroups = BTreeMap::new();
    for paths in paths_by_size.into_values().filter(|paths| paths.len() > 1) {
        for path in paths {
            if should_cancel() {
                report_progress(progress);
                return Ok(DuplicateScanOutcome::Cancelled);
            }
            match file_hash_with_cancel(&path, &mut should_cancel) {
                Ok(Some(hash)) => paths_by_hash.entry(hash).or_default().push(path),
                Ok(None) => {
                    report_progress(progress);
                    return Ok(DuplicateScanOutcome::Cancelled);
                }
                Err(_) => {}
            }
            progress.files_hashed += 1;
            if progress.files_hashed % 25 == 0 {
                report_progress(progress.clone());
            }
        }
    }

    for paths in paths_by_hash.values_mut() {
        paths.sort();
    }
    paths_by_hash.retain(|_, paths| paths.len() > 1);
    progress.duplicate_groups = paths_by_hash.len();
    report_progress(progress);
    Ok(DuplicateScanOutcome::Completed(paths_by_hash))
}

pub(crate) fn require_scan_roots(roots: &[PathBuf]) -> io::Result<()> {
    if roots.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Select at least one scan location.",
        ))
    } else {
        Ok(())
    }
}

/// Produce a plain-text summary suitable for copying or saving alongside a scan.
pub fn format_scan_report(
    roots: &[PathBuf],
    progress: &ScanProgress,
    groups: &DuplicateGroups,
) -> String {
    let duplicate_files: usize = groups.values().map(Vec::len).sum();
    format!(
        "Exact media scan report\n\nLocations:\n{}\n\nFolders visited: {}\nFiles visited: {}\nKnown media files: {}\nFiles hashed this scan: {}\nSaved hashes reused: {}\nDuplicate groups: {}\nDuplicate files: {}\nExtra copies: {}\n",
        roots
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n"),
        progress.directories_visited,
        progress.files_visited,
        progress.media_files_found,
        progress.files_hashed,
        progress.hashes_reused,
        groups.len(),
        duplicate_files,
        duplicate_files.saturating_sub(groups.len())
    )
}

/// Return local fixed, removable, and network drive roots according to the selected options.
#[cfg(windows)]
pub fn computer_scan_roots(include_removable: bool, include_network: bool) -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    let mask = unsafe { GetLogicalDrives() };
    let mut roots = Vec::new();
    for letter in b'A'..=b'Z' {
        if mask & (1 << (letter - b'A')) == 0 {
            continue;
        }
        let wide = [letter as u16, b':' as u16, b'\\' as u16, 0];
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if kind == DRIVE_FIXED
            || (include_removable && kind == DRIVE_REMOVABLE)
            || (include_network && kind == DRIVE_REMOTE)
        {
            roots.push(PathBuf::from(format!("{}:\\", letter as char)));
        }
    }
    roots
}

#[cfg(not(windows))]
pub fn computer_scan_roots(_: bool, _: bool) -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

/// Return paths that may be actioned: all unkept files only when every group retains a keeper.
pub fn unkept_duplicate_paths(
    groups: &DuplicateGroups,
    kept: &HashSet<PathBuf>,
) -> Result<Vec<PathBuf>, String> {
    let mut unkept = Vec::new();
    for paths in groups.values() {
        if !paths.iter().any(|path| kept.contains(path)) {
            return Err("Each duplicate group must retain at least one item.".to_owned());
        }
        unkept.extend(paths.iter().filter(|path| !kept.contains(*path)).cloned());
    }
    Ok(unkept)
}

/// Return explicitly selected duplicate paths when every group still retains a copy.
///
/// Unlike [`unkept_duplicate_paths`], this representation stays empty for the safe default where
/// every file is kept. That avoids duplicating millions of paths in memory after a large scan.
pub fn selected_duplicate_paths(
    groups: &DuplicateGroups,
    selected: &HashSet<PathBuf>,
) -> Result<Vec<PathBuf>, String> {
    for paths in groups.values() {
        if paths.iter().all(|path| selected.contains(path)) {
            return Err("Each duplicate group must retain at least one item.".to_owned());
        }
    }
    Ok(groups
        .values()
        .flatten()
        .filter(|path| selected.contains(*path))
        .cloned()
        .collect())
}

/// Move files to the operating system Recycle Bin.
pub fn recycle_files(paths: &[PathBuf]) -> FileActionSummary {
    apply_files(paths, |path| {
        trash::delete(path).map_err(|error| error.to_string())
    })
}

/// Permanently delete files. This operation cannot be undone.
pub fn permanently_delete_files(paths: &[PathBuf]) -> FileActionSummary {
    apply_files(paths, |path| {
        fs::remove_file(path).map_err(|error| error.to_string())
    })
}

/// Create a ZIP backup of files with a manifest preserving their original locations.
pub fn archive_files(
    paths: &[PathBuf],
    destination: &Path,
    compression: ArchiveCompression,
) -> Result<FileActionSummary, String> {
    if destination.exists() {
        return Err(format!("Archive already exists: {}", destination.display()));
    }
    let file = File::create(destination).map_err(|error| error.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = zip_options(compression);
    let mut manifest = Vec::new();
    let mut summary = FileActionSummary {
        processed: 0,
        failures: Vec::new(),
    };
    for (index, path) in paths.iter().enumerate() {
        let archive_path = format!(
            "media/{index:05}_{}",
            path.file_name().and_then(OsStr::to_str).unwrap_or("file")
        );
        match (File::open(path), zip.start_file(&archive_path, options)) {
            (Ok(mut input), Ok(())) => match io::copy(&mut input, &mut zip) {
                Ok(_) => {
                    summary.processed += 1;
                    manifest.push(ArchiveEntry {
                        original_path: path.display().to_string(),
                        archive_path,
                    });
                }
                Err(error) => summary
                    .failures
                    .push(format!("{}: {error}", path.display())),
            },
            (Err(error), _) => summary
                .failures
                .push(format!("{}: {error}", path.display())),
            (_, Err(error)) => summary
                .failures
                .push(format!("{}: {error}", path.display())),
        }
    }
    let manifest_json = serde_json::to_vec_pretty(&ArchiveManifest { files: manifest })
        .map_err(|error| error.to_string())?;
    zip.start_file("manifest.json", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(&manifest_json)
        .map_err(|error| error.to_string())?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok(summary)
}

#[derive(Serialize)]
struct ArchiveManifest {
    files: Vec<ArchiveEntry>,
}
#[derive(Serialize)]
struct ArchiveEntry {
    original_path: String,
    archive_path: String,
}

fn apply_files(
    paths: &[PathBuf],
    action: impl Fn(&Path) -> Result<(), String>,
) -> FileActionSummary {
    let mut summary = FileActionSummary {
        processed: 0,
        failures: Vec::new(),
    };
    for path in paths {
        match action(path) {
            Ok(()) => summary.processed += 1,
            Err(error) => summary
                .failures
                .push(format!("{}: {error}", path.display())),
        }
    }
    summary
}

fn zip_options(compression: ArchiveCompression) -> SimpleFileOptions {
    match compression {
        ArchiveCompression::Stored => {
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
        }
        ArchiveCompression::Fast => SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(1)),
        ArchiveCompression::Default => {
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
        }
        ArchiveCompression::Maximum => SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9)),
    }
}

/// Move images with an EXIF `DateTimeOriginal` into `Year/Mon` folders.
///
/// Returns the number of files that were moved. Existing destination files are
/// preserved by appending `_1`, `_2`, and so on to the new filename.
pub fn sort_images(directory: &Path) -> io::Result<usize> {
    let root = absolute_path(directory)?;
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Directory does not exist: {}", directory.display()),
        ));
    }

    let mut moved = 0;
    for source in image_paths(&root)? {
        let Some((year, month)) = date_folder(&source) else {
            continue;
        };
        let Some(file_name) = source.file_name() else {
            continue;
        };

        let destination_dir = root.join(year.to_string()).join(month);
        fs::create_dir_all(&destination_dir)?;
        let destination = available_destination(&destination_dir, file_name);
        if source == destination {
            continue;
        }

        fs::rename(source, destination)?;
        moved += 1;
    }
    Ok(moved)
}

/// Prefix supported media files recursively with their preferred date.
///
/// Embedded image capture dates take precedence over filesystem timestamps.
/// Existing `YYYY-MM-DD - ` prefixes are preserved unless correction is enabled.
pub fn prefix_media_files_with_date(
    directory: &Path,
    options: DatePrefixOptions,
) -> Result<RenameSummary, String> {
    if !directory.is_dir() {
        return Err(format!("Directory does not exist: {}", directory.display()));
    }
    let root = absolute_path(directory).map_err(|error| error.to_string())?;
    let mut summary = RenameSummary {
        renamed: 0,
        corrected_prefixes: 0,
        capture_dates_used: 0,
        filesystem_dates_used: 0,
        skipped: 0,
        failures: Vec::new(),
    };
    for source in media_paths(&[root]).map_err(|error| error.to_string())? {
        let Some(file_name) = source.file_name().and_then(OsStr::to_str) else {
            summary.skipped += 1;
            continue;
        };
        let existing_prefix = has_date_prefix(file_name);
        if existing_prefix && !options.correct_existing_prefixes {
            summary.skipped += 1;
            continue;
        }
        let preferred_date = match preferred_media_date(&source, options) {
            Ok(date) => date,
            Err(error) => {
                summary
                    .failures
                    .push(format!("{}: {error}", source.display()));
                continue;
            }
        };
        if existing_prefix && file_name.starts_with(&preferred_date.text) {
            summary.skipped += 1;
            continue;
        }
        let Some(parent) = source.parent() else {
            summary
                .failures
                .push(format!("{}: no parent directory", source.display()));
            continue;
        };
        let unprefixed_name = if existing_prefix {
            &file_name[13..]
        } else {
            file_name
        };
        let destination = available_destination(
            parent,
            OsStr::new(&format!("{} - {unprefixed_name}", preferred_date.text)),
        );
        match fs::rename(&source, destination) {
            Ok(()) => {
                summary.renamed += 1;
                summary.corrected_prefixes += usize::from(existing_prefix);
                match preferred_date.source {
                    FilenameDateSource::EmbeddedCapture => summary.capture_dates_used += 1,
                    FilenameDateSource::Filesystem => summary.filesystem_dates_used += 1,
                }
            }
            Err(error) => summary
                .failures
                .push(format!("{}: {error}", source.display())),
        }
    }
    Ok(summary)
}

/// Apply a modest, local enhancement to a photograph and write a new sibling file.
///
/// The original is never changed. The default output name is `name (Enhanced).ext`;
/// a numbered suffix is used if that file already exists.
pub fn enhance_photo(input: &Path) -> Result<PathBuf, String> {
    if !input.is_file() {
        return Err(format!("Image does not exist: {}", input.display()));
    }
    let output = enhanced_destination(input)?;
    let original = image::open(input)
        .map_err(|error| error.to_string())?
        .to_rgb8();
    let cleaned = median_filter_3x3(&original);
    let enhanced = conservative_enhancement(&original, &cleaned);
    save_rgb_image(&enhanced, &output)?;
    Ok(output)
}

/// Produce the restoration prompt with the selected filename as context.
pub fn restoration_prompt(path: &Path) -> String {
    format!(
        "AI RESTORATION PROMPT FOR: {}\n\n{RESTORATION_PROMPT}",
        path.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("selected photo")
    )
}

/// Make duplicate results readable in the GUI and command-line output.
pub fn format_duplicate_report(duplicates: &DuplicateGroups) -> String {
    if duplicates.is_empty() {
        return "No exact duplicate images were found.".to_owned();
    }

    let extra_copies: usize = duplicates.values().map(|paths| paths.len() - 1).sum();
    let mut lines = vec![format!(
        "Found {} duplicate group(s) ({extra_copies} extra copy/copies).",
        duplicates.len()
    )];
    for (index, paths) in duplicates.values().enumerate() {
        lines.push(String::new());
        lines.push(format!("Group {}:", index + 1));
        lines.extend(paths.iter().map(|path| format!("  {}", path.display())));
    }
    lines.join("\n")
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn has_date_prefix(file_name: &str) -> bool {
    let bytes = file_name.as_bytes();
    bytes.len() >= 13
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b' '
        && bytes[11] == b'-'
        && bytes[12] == b' '
}

fn filesystem_date(path: &Path, use_oldest_date: bool) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    let created = metadata.created().or_else(|_| metadata.modified())?;
    let date = if use_oldest_date {
        match metadata.modified() {
            Ok(modified) if modified < created => modified,
            _ => created,
        }
    } else {
        created
    };
    let local: DateTime<Local> = date.into();
    Ok(local.format("%Y-%m-%d").to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilenameDateSource {
    EmbeddedCapture,
    Filesystem,
}

struct FilenameDate {
    text: String,
    source: FilenameDateSource,
}

fn preferred_media_date(path: &Path, options: DatePrefixOptions) -> io::Result<FilenameDate> {
    if media_kind(path) == Some(MediaKind::Image)
        && let Some(date) = exif_capture_date(path)
    {
        return Ok(FilenameDate {
            text: date.format("%Y-%m-%d").to_string(),
            source: FilenameDateSource::EmbeddedCapture,
        });
    }
    Ok(FilenameDate {
        text: filesystem_date(path, options.use_oldest_filesystem_date)?,
        source: FilenameDateSource::Filesystem,
    })
}

fn enhanced_destination(input: &Path) -> Result<PathBuf, String> {
    let parent = input
        .parent()
        .ok_or_else(|| format!("Image has no parent directory: {}", input.display()))?;
    let stem = input
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("Image has no valid filename: {}", input.display()))?;
    let extension = input
        .extension()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("Image has no extension: {}", input.display()))?;
    Ok(available_destination(
        parent,
        OsStr::new(&format!("{stem} (Enhanced).{extension}")),
    ))
}

fn median_filter_3x3(input: &RgbImage) -> RgbImage {
    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut channels = [[0_u8; 9]; 3];
            let mut index = 0;
            for sample_y in y.saturating_sub(1)..=(y + 1).min(height.saturating_sub(1)) {
                for sample_x in x.saturating_sub(1)..=(x + 1).min(width.saturating_sub(1)) {
                    let pixel = input.get_pixel(sample_x, sample_y);
                    for channel in 0..3 {
                        channels[channel][index] = pixel[channel];
                    }
                    index += 1;
                }
            }
            for values in &mut channels {
                values[..index].sort_unstable();
            }
            output.put_pixel(
                x,
                y,
                Rgb([
                    channels[0][index / 2],
                    channels[1][index / 2],
                    channels[2][index / 2],
                ]),
            );
        }
    }
    output
}

fn conservative_enhancement(original: &RgbImage, cleaned: &RgbImage) -> RgbImage {
    let (width, height) = original.dimensions();
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let source = original.get_pixel(x, y);
            let filtered = cleaned.get_pixel(x, y);
            let mut pixel = [0_u8; 3];
            for channel in 0..3 {
                pixel[channel] =
                    ((u16::from(source[channel]) * 3 + u16::from(filtered[channel])) / 4) as u8;
            }
            let luminance = 0.299 * f32::from(pixel[0])
                + 0.587 * f32::from(pixel[1])
                + 0.114 * f32::from(pixel[2]);
            for value in &mut pixel {
                let saturated = luminance + (f32::from(*value) - luminance) * 1.10;
                *value = ((saturated - 128.0) * 1.15 + 128.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            output.put_pixel(x, y, Rgb(pixel));
        }
    }
    output
}

fn save_rgb_image(image: &RgbImage, output: &Path) -> Result<(), String> {
    let format = match output
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => ImageFormat::Png,
        Some("bmp") => ImageFormat::Bmp,
        Some("tif" | "tiff") => ImageFormat::Tiff,
        _ => {
            return Err(format!(
                "This build can enhance PNG, BMP, and TIFF images; unsupported output: {}",
                output.display()
            ));
        }
    };
    let file = File::create(output).map_err(|error| error.to_string())?;
    image
        .write_to(&mut io::BufWriter::new(file), format)
        .map_err(|error| error.to_string())
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn date_folder(path: &Path) -> Option<(u16, &'static str)> {
    let date = exif_capture_date(path)?;
    let year = u16::try_from(date.year()).ok()?;
    Some((year, month_name(date.month())?))
}

fn exif_capture_date(path: &Path) -> Option<NaiveDate> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = Reader::new().read_from_container(&mut reader).ok()?;
    [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .into_iter()
        .filter_map(|tag| exif.get_field(tag, In::PRIMARY))
        .find_map(|field| {
            let Value::Ascii(values) = &field.value else {
                return None;
            };
            values
                .iter()
                .filter_map(|value| std::str::from_utf8(value).ok())
                .find_map(parse_exif_date)
        })
}

fn parse_exif_date(value: &str) -> Option<NaiveDate> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes[4] != b':' || bytes[7] != b':' {
        return None;
    }
    NaiveDate::from_ymd_opt(
        value[..4].parse().ok()?,
        value[5..7].parse().ok()?,
        value[8..10].parse().ok()?,
    )
}

fn month_name(month: u32) -> Option<&'static str> {
    match month {
        1 => Some("Jan"),
        2 => Some("Feb"),
        3 => Some("Mar"),
        4 => Some("Apr"),
        5 => Some("May"),
        6 => Some("Jun"),
        7 => Some("Jul"),
        8 => Some("Aug"),
        9 => Some("Sep"),
        10 => Some("Oct"),
        11 => Some("Nov"),
        12 => Some("Dec"),
        _ => None,
    }
}

fn available_destination(directory: &Path, file_name: &OsStr) -> PathBuf {
    let requested = directory.join(file_name);
    if !requested.exists() {
        return requested;
    }

    let source = Path::new(file_name);
    let stem = source
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("image");
    let extension = source.extension().and_then(OsStr::to_str);
    for counter in 1.. {
        let name = match extension {
            Some(extension) => format!("{stem}_{counter}.{extension}"),
            None => format!("{stem}_{counter}"),
        };
        let candidate = directory.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the counter is unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("media-sift-{unique}"));
        fs::create_dir_all(&path).expect("create temporary directory");
        path
    }

    fn jpeg_with_exif_date(date: &str) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42_u16.to_le_bytes());
        tiff.extend_from_slice(&8_u32.to_le_bytes());

        // IFD0 with an ExifIFDPointer to the EXIF IFD at byte 26.
        tiff.extend_from_slice(&1_u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769_u16.to_le_bytes());
        tiff.extend_from_slice(&4_u16.to_le_bytes());
        tiff.extend_from_slice(&1_u32.to_le_bytes());
        tiff.extend_from_slice(&26_u32.to_le_bytes());
        tiff.extend_from_slice(&0_u32.to_le_bytes());

        // EXIF IFD with DateTimeOriginal, whose NUL-terminated data starts at byte 44.
        tiff.extend_from_slice(&1_u16.to_le_bytes());
        tiff.extend_from_slice(&0x9003_u16.to_le_bytes());
        tiff.extend_from_slice(&2_u16.to_le_bytes());
        tiff.extend_from_slice(&((date.len() + 1) as u32).to_le_bytes());
        tiff.extend_from_slice(&44_u32.to_le_bytes());
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff.extend_from_slice(date.as_bytes());
        tiff.push(0);

        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }

    #[test]
    fn recognises_image_extensions_case_insensitively() {
        assert!(is_image_path(Path::new("photo.JPEG")));
        assert!(is_image_path(Path::new("photo.png")));
        assert!(!is_image_path(Path::new("notes.txt")));
    }

    #[test]
    fn hashes_identical_content_equally() {
        let root = temp_dir();
        let first = root.join("first.jpg");
        let second = root.join("second.jpg");
        fs::write(&first, b"same image bytes").expect("write first image");
        fs::copy(&first, &second).expect("copy image");

        assert_eq!(file_hash(&first).unwrap(), file_hash(&second).unwrap());
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    fn two_path_duplicate_group() -> (DuplicateGroups, PathBuf, PathBuf) {
        let first = PathBuf::from("one.jpg");
        let second = PathBuf::from("two.jpg");
        let groups =
            DuplicateGroups::from([("hash".to_owned(), vec![first.clone(), second.clone()])]);
        (groups, first, second)
    }

    #[test]
    fn destructive_verification_detects_same_size_content_changes() {
        let root = temp_dir();
        let keeper = root.join("keeper.jpg");
        let selected = root.join("selected.jpg");
        fs::write(&keeper, b"0123456789abcdef").unwrap();
        fs::write(&selected, b"0123456789abcdef").unwrap();
        let verification = DuplicateActionVerification {
            expected_hash: file_hash(&keeper).unwrap(),
            keeper,
            selected: vec![selected.clone()],
        };

        verify_duplicate_action_groups(std::slice::from_ref(&verification)).unwrap();
        fs::write(selected, b"fedcba9876543210").unwrap();
        assert!(verify_duplicate_action_groups(&[verification]).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_exact_duplicates_recursively() {
        let root = temp_dir();
        let nested = root.join("nested");
        fs::create_dir(&nested).expect("create nested directory");
        let original = root.join("original.jpg");
        let copy = nested.join("copy.JPG");
        fs::write(&original, b"same image bytes").expect("write image");
        fs::copy(&original, &copy).expect("copy image");
        fs::write(nested.join("notes.txt"), b"same image bytes").expect("write text file");

        let groups = find_exact_duplicates(&root).unwrap();
        assert_eq!(groups.len(), 1);
        let mut expected = vec![original, copy];
        expected.sort();
        assert_eq!(groups.values().next().unwrap(), &expected);
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn does_not_report_same_size_different_content() {
        let root = temp_dir();
        fs::write(root.join("first.jpg"), b"abc").expect("write first image");
        fs::write(root.join("second.jpg"), b"def").expect("write second image");

        assert!(find_exact_duplicates(&root).unwrap().is_empty());
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn duplicate_scan_can_be_cancelled_cooperatively() {
        let root = temp_dir();
        fs::write(root.join("first.jpg"), b"same image bytes").expect("write first image");
        fs::write(root.join("second.jpg"), b"same image bytes").expect("write second image");
        let mut cancellation_checks = 0;
        let mut progress_updates = Vec::new();

        let outcome = find_exact_duplicates_with_progress_and_cancel(
            std::slice::from_ref(&root),
            |progress| progress_updates.push(progress),
            || {
                cancellation_checks += 1;
                cancellation_checks >= 3
            },
        )
        .expect("cancel scan cleanly");

        assert_eq!(outcome, DuplicateScanOutcome::Cancelled);
        assert!(!progress_updates.is_empty());
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn overlapping_scan_roots_do_not_count_the_same_file_twice() {
        let root = temp_dir();
        let nested = root.join("nested");
        fs::create_dir(&nested).expect("create nested directory");
        fs::write(root.join("first.jpg"), b"same image bytes").expect("write first image");
        fs::write(nested.join("second.jpg"), b"same image bytes").expect("write second image");
        let mut final_progress = ScanProgress::default();

        let outcome = find_exact_duplicates_with_progress_and_cancel(
            &[root.clone(), nested],
            |progress| final_progress = progress,
            || false,
        )
        .expect("scan overlapping roots");

        let DuplicateScanOutcome::Completed(groups) = outcome else {
            panic!("scan should complete");
        };
        assert_eq!(groups.len(), 1);
        assert_eq!(groups.values().next().expect("duplicate group").len(), 2);
        assert_eq!(final_progress.media_files_found, 2);
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn parses_valid_exif_dates() {
        assert_eq!(
            parse_exif_date("2024:01:02 03:04:05"),
            NaiveDate::from_ymd_opt(2024, 1, 2)
        );
        assert_eq!(parse_exif_date("2024:13:02 03:04:05"), None);
        assert_eq!(parse_exif_date("2024:02:30 03:04:05"), None);
        assert_eq!(parse_exif_date("not a date"), None);
    }

    #[test]
    fn sorts_exif_images_and_preserves_filename_collisions() {
        let root = temp_dir();
        let first_dir = root.join("first");
        let second_dir = root.join("second");
        fs::create_dir(&first_dir).expect("create first directory");
        fs::create_dir(&second_dir).expect("create second directory");
        let image = jpeg_with_exif_date("2024:01:02 03:04:05");
        fs::write(first_dir.join("photo.jpg"), &image).expect("write first image");
        fs::write(second_dir.join("photo.jpg"), image).expect("write second image");

        assert_eq!(sort_images(&root).unwrap(), 2);
        let destination = root.join("2024").join("Jan");
        assert!(destination.join("photo.jpg").is_file());
        assert!(destination.join("photo_1.jpg").is_file());
        fs::remove_dir_all(root).expect("remove temporary directory");
    }

    #[test]
    fn report_lists_groups_and_extra_copies() {
        let mut groups = DuplicateGroups::new();
        groups.insert(
            "hash".to_owned(),
            vec![PathBuf::from("one.jpg"), PathBuf::from("two.jpg")],
        );

        let report = format_duplicate_report(&groups);
        assert!(report.contains("1 duplicate group(s) (1 extra copy/copies)"));
        assert!(report.contains("one.jpg"));
        assert!(report.contains("two.jpg"));
    }

    #[test]
    fn scans_known_video_and_audio_but_ignores_unknown_files() {
        let root = temp_dir();
        fs::write(root.join("one.mp4"), b"media bytes").unwrap();
        fs::write(root.join("two.mkv"), b"media bytes").unwrap();
        fs::write(root.join("notes.bin"), b"media bytes").unwrap();
        assert_eq!(media_kind(&root.join("one.mp4")), Some(MediaKind::Video));
        assert_eq!(media_kind(&root.join("song.flac")), Some(MediaKind::Audio));
        assert_eq!(find_exact_duplicates(&root).unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unkept_paths_require_a_keeper_in_every_group() {
        let (groups, first, second) = two_path_duplicate_group();
        assert!(unkept_duplicate_paths(&groups, &HashSet::new()).is_err());
        let kept = HashSet::from([first]);
        assert_eq!(
            unkept_duplicate_paths(&groups, &kept).unwrap(),
            vec![second]
        );
    }

    #[test]
    fn selected_paths_are_sparse_and_require_a_keeper_in_every_group() {
        let (groups, first, second) = two_path_duplicate_group();

        assert_eq!(
            selected_duplicate_paths(&groups, &HashSet::new()).unwrap(),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            selected_duplicate_paths(&groups, &HashSet::from([second.clone()])).unwrap(),
            vec![second.clone()]
        );
        assert!(selected_duplicate_paths(&groups, &HashSet::from([first, second])).is_err());
    }

    #[test]
    fn identifies_existing_date_prefixes() {
        assert!(has_date_prefix("2024-01-02 - photo.jpg"));
        assert!(!has_date_prefix("2024-1-02 - photo.jpg"));
        assert!(!has_date_prefix("holiday photo.jpg"));
    }

    #[test]
    fn prefixes_known_media_files_without_touching_other_files() {
        let root = temp_dir();
        fs::write(root.join("photo.jpg"), b"jpeg bytes").unwrap();
        fs::write(root.join("notes.txt"), b"notes").unwrap();

        let summary = prefix_media_files_with_date(&root, DatePrefixOptions::default()).unwrap();
        assert_eq!(summary.renamed, 1);
        assert_eq!(summary.filesystem_dates_used, 1);
        assert_eq!(summary.failures, Vec::<String>::new());
        assert!(root.join("notes.txt").is_file());
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(" - photo.jpg"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filename_prefix_prefers_embedded_date_taken_over_filesystem_date() {
        let root = temp_dir();
        let source = root.join("A Tree Over the River.jpg");
        fs::write(&source, jpeg_with_exif_date("2010:09:20 15:55:00")).unwrap();

        let summary = prefix_media_files_with_date(&root, DatePrefixOptions::default()).unwrap();

        assert_eq!(summary.renamed, 1);
        assert_eq!(summary.capture_dates_used, 1);
        assert_eq!(summary.filesystem_dates_used, 0);
        assert!(
            root.join("2010-09-20 - A Tree Over the River.jpg")
                .is_file()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incorrect_existing_prefix_requires_explicit_correction() {
        let root = temp_dir();
        let incorrect = root.join("2017-07-06 - A Tree Over the River.jpg");
        fs::write(&incorrect, jpeg_with_exif_date("2010:09:20 15:55:00")).unwrap();

        let preserved = prefix_media_files_with_date(&root, DatePrefixOptions::default()).unwrap();
        assert_eq!(preserved.skipped, 1);
        assert!(incorrect.is_file());

        let corrected = prefix_media_files_with_date(
            &root,
            DatePrefixOptions {
                correct_existing_prefixes: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(corrected.renamed, 1);
        assert_eq!(corrected.corrected_prefixes, 1);
        assert!(
            root.join("2010-09-20 - A Tree Over the River.jpg")
                .is_file()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enhancement_creates_a_new_photo_without_replacing_source() {
        let root = temp_dir();
        let input = root.join("portrait.png");
        RgbImage::from_pixel(3, 3, Rgb([120, 90, 70]))
            .save(&input)
            .unwrap();

        let output = enhance_photo(&input).unwrap();
        assert!(input.is_file());
        assert!(output.is_file());
        assert_ne!(input, output);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restoration_prompt_names_selected_photo() {
        let prompt = restoration_prompt(Path::new("archive/grandmother.jpg"));
        assert!(prompt.contains("grandmother.jpg"));
        assert!(prompt.contains("Do NOT alter"));
    }
}
