//! Core file-system operations for Image Sorter.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use exif::{In, Reader, Tag, Value};
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

/// Duplicate paths grouped by their SHA-256 file-content hash.
pub type DuplicateGroups = BTreeMap<String, Vec<PathBuf>>;

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
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; HASH_CHUNK_SIZE];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
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
    let mut paths_by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in media_paths(roots)? {
        if let Ok(metadata) = fs::metadata(&path) {
            paths_by_size.entry(metadata.len()).or_default().push(path);
        }
    }

    let mut paths_by_hash: DuplicateGroups = BTreeMap::new();
    for paths in paths_by_size.into_values().filter(|paths| paths.len() > 1) {
        for path in paths {
            if let Ok(hash) = file_hash(&path) {
                paths_by_hash.entry(hash).or_default().push(path);
            }
        }
    }

    for paths in paths_by_hash.values_mut() {
        paths.sort();
    }
    paths_by_hash.retain(|_, paths| paths.len() > 1);
    Ok(paths_by_hash)
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

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn date_folder(path: &Path) -> Option<(u16, &'static str)> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = Reader::new().read_from_container(&mut reader).ok()?;
    let field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY)?;
    let Value::Ascii(values) = &field.value else {
        return None;
    };
    let date = std::str::from_utf8(values.first()?).ok()?;
    parse_exif_date(date)
}

fn parse_exif_date(value: &str) -> Option<(u16, &'static str)> {
    let bytes = value.as_bytes();
    if bytes.len() < 7 || bytes[4] != b':' {
        return None;
    }
    let year = value[..4].parse().ok()?;
    let month = match &value[5..7] {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return None,
    };
    Some((year, month))
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
        let path = std::env::temp_dir().join(format!("image-sorter-{unique}"));
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
    fn parses_valid_exif_dates() {
        assert_eq!(parse_exif_date("2024:01:02 03:04:05"), Some((2024, "Jan")));
        assert_eq!(parse_exif_date("2024:13:02 03:04:05"), None);
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
        let first = PathBuf::from("one.jpg");
        let second = PathBuf::from("two.jpg");
        let mut groups = DuplicateGroups::new();
        groups.insert("hash".to_owned(), vec![first.clone(), second.clone()]);
        assert!(unkept_duplicate_paths(&groups, &HashSet::new()).is_err());
        let kept = HashSet::from([first]);
        assert_eq!(
            unkept_duplicate_paths(&groups, &kept).unwrap(),
            vec![second]
        );
    }
}
