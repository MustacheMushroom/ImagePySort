# MediaSift

MediaSift is a Rust desktop utility for organizing images by capture date and
reviewing exact duplicate image, video, and audio files.

## Install on Windows

Run the Windows setup executable from a release artifact or build it locally
with:

```powershell
cargo install cargo-packager --version 0.11.8 --locked
.\scripts\Build-WindowsInstaller.ps1
```

The installer defaults to `C:\Program Files\MediaSift`, lets you choose a
different destination, creates a searchable **MediaSift** Start Menu shortcut,
and registers a standard uninstaller in Windows Installed apps. It does not add
file associations, startup items, services, or application-settings registry
keys. See [the Windows installer guide](doc/WINDOWS_INSTALLER.md) for packaging
and registry details.

## Run from source

Install the [Rust toolchain](https://www.rust-lang.org/tools/install), then run:

```powershell
cargo run --release
```

The interface is organized into four workflows:

- **Overview** explains what each workflow changes and links to the right tool.
- **Duplicate finder** scans local fixed drives by default only when no folders
  are selected. You can select several folders at once or add more folders in a
  later pass; when the list is non-empty, only those folders are scanned.
  Removable-drive and network-drive options apply to the automatic drive scope.
  Scans are read-only and cancellable; every match starts marked **Keep**, and
  file actions stay disabled until at least one copy in every group is protected.
- **Organize** can sort photos into `Year/Month` folders using EXIF
  `DateTimeOriginal` information. It can also prefix media filenames with a
  `YYYY-MM-DD` plus `-` filesystem-date
  prefix to known media files below a chosen folder. The app skips already
  prefixed files, prevents filename collisions, and asks for confirmation before
  either operation changes files. Each organizer can optionally open the
  selected result folder in File Explorer after a successful run.
- **Photo tools** can create a separate PNG, BMP, or TIFF sibling image with an
  `(Enhanced)` filename suffix using mild local dust/noise cleanup, contrast,
  and color adjustments. It can also copy a conservative, identity-preserving
  prompt for use with an external image service.

Use `Alt+1` through `Alt+4` to move between workflows. The Theme menu can follow
Windows or explicitly use light or dark mode.

During a scan, the interface shows live folder, file, media-file, hash, and
duplicate-group counts plus a **Cancel scan** action. Cancellation is checked
throughout directory traversal and file hashing, and never changes files. Once
results are ready, groups are paginated in batches of 50 so even very large
reviews remain responsive. Keep selections can be changed individually, by
source root, or with a safe bulk choice. The safe default stores no duplicate
path selections in memory; only files explicitly chosen for an action are
tracked. Unchecked files can be
sent to the Recycle Bin, permanently deleted, or archived in a ZIP file that
records their original locations. Confirmations show the exact number of files
affected. After recycling or deleting files, the old review is cleared so it
cannot accidentally be reused.

The desktop interface runs scans and sorting in a background thread. Completed
duplicate reviews cache their summary counts and render a bounded page rather
than rebuilding every result row on each frame.

The native application is split into focused state, background-operation, page,
dialog, and reusable-component modules. See
[the architecture guide](doc/ARCHITECTURE.md) for module responsibilities and
dependency rules.

The original imported-script assessment and integration boundary are documented
in [doc/UTILITY_SCRIPT_INVENTORY.md](doc/UTILITY_SCRIPT_INVENTORY.md).

## Development checks

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```
