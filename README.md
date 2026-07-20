# MediaSift

MediaSift is a Rust desktop utility for organizing images by capture date and
reviewing exact duplicate image, video, and audio files.

## Run

Install the [Rust toolchain](https://www.rust-lang.org/tools/install), then run:

```powershell
cargo run --release
```

Choose either:

- **Select Directory to Sort** to organize photos into `Year/Month` folders
  using EXIF `DateTimeOriginal` information.
- **Scan for Exact Duplicates** to scan local fixed drives by default, then
  optionally add removable drives, network drives, or custom folders. Only
  supported media formats are hashed. Exact duplicates are identified by their
  SHA-256 content hash; visually similar images or differently encoded copies
  are not reported.

During a scan, the interface shows live folder, file, media-file, hash, and
duplicate-group counts. Once results are ready, keep selections can be changed
individually or in bulk by source root. Unkept files can be sent to the Recycle
Bin, permanently deleted, or archived in a ZIP file that records their
original locations. The scan itself is read-only.

The desktop interface runs scans and sorting in a background thread, so it
remains responsive while it processes a large folder or drive.

## Development checks

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```
