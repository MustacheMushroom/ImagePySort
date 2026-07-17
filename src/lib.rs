//! Core file-system operations for Image Sorter.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
};

use exif::{In, Reader, Tag, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

/// Image extensions supported by both sorting and duplicate scanning.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp"];
const HASH_CHUNK_SIZE: usize = 1024 * 1024;

/// Duplicate paths grouped by their SHA-256 file-content hash.
pub type DuplicateGroups = BTreeMap<String, Vec<PathBuf>>;

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
    let mut paths_by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in image_paths(directory)? {
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
}
