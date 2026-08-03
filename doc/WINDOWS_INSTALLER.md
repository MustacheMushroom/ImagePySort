# Windows installer

MediaSift is packaged as an NSIS installer using `cargo-packager` 0.11.8. The
installer is per-machine by default, so its suggested destination is
`C:\Program Files\MediaSift` and Windows requests administrator approval. The
directory page remains available, so a user can choose another destination.

The installer creates a **MediaSift** Start Menu shortcut and adds MediaSift to
Windows' Installed apps list. It does not create file associations, shell
extensions, startup entries, background services, or application-settings
registry keys.

The application mark is embedded directly in `media-sift.exe`, supplied to the
live eframe window, and used for the setup executable. Its multi-resolution ICO
includes exact Windows shell sizes so Start, search, the taskbar, Explorer, and
Installed apps do not have to upscale a small bitmap.

## Build locally

Install the pinned packaging tool once:

```powershell
cargo install cargo-packager --version 0.11.8 --locked
```

Then build the release executable and installer:

```powershell
.\scripts\Build-WindowsInstaller.ps1
```

The generated setup executable is written below `target\release`. Run it and
use the directory page to accept Program Files or choose a custom location.
Locally built installers are unsigned, so Windows SmartScreen may ask for an
extra confirmation. Production release installers should be Authenticode-signed
when a signing certificate is available.

## Registry scope

NSIS writes only conventional installer bookkeeping for this configuration:

- the selected install location, used by upgrades and reinstalls;
- the Windows Installed apps/uninstall entry;
- the installer UI language.

Per-machine entries are stored under `HKLM`; the installer-language preference
is stored under `HKCU`. This metadata makes reliable upgrades and uninstallation
possible. MediaSift itself continues to use no registry-backed configuration.

## Release automation

`.github/workflows/windows-installer.yml` builds the installer for version tags
and can also be run manually. It publishes the setup executable as a workflow
artifact. The tool version is pinned in both the workflow and local build script
so local and CI packages use the same installer generator.
