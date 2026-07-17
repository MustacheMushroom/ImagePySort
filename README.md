# ImagePySort
A Rust desktop utility to organize images in folders based on the date taken
and find exact duplicate images.

## Run

Install the [Rust toolchain](https://www.rust-lang.org/tools/install), then run:

```powershell
cargo run --release
```

Choose either:

- **Select Directory to Sort** to organize photos into `Year/Month` folders
  using EXIF `DateTimeOriginal` information.
- **Scan for Exact Duplicates** to select a folder or drive root (for example,
  `D:\\`). Duplicate images are identified by their SHA-256 content hash;
  visually similar images or differently encoded copies are not reported. The
  scan is read-only and does not delete or move any files.

The desktop interface runs scans and sorting in a background thread, so it
remains responsive while it processes a large folder or drive.

## Development checks

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```
