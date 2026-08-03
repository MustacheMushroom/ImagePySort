[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$requiredPackagerVersion = "0.11.8"

if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    throw @"
cargo-packager is required to build the installer.
Install the repository's pinned version with:
  cargo install cargo-packager --version $requiredPackagerVersion --locked
"@
}

$versionOutput = (& cargo-packager --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Unable to determine the cargo-packager version: $versionOutput"
}

if ($versionOutput -notmatch "(^|\s)$([regex]::Escape($requiredPackagerVersion))(\s|$)") {
    throw "Expected cargo-packager $requiredPackagerVersion, but found '$versionOutput'."
}

& cargo packager --release --formats nsis
if ($LASTEXITCODE -ne 0) {
    throw "cargo-packager failed with exit code $LASTEXITCODE."
}

$installer = Get-ChildItem -Path "target\release" -Filter "media-sift_*-setup.exe" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $installer) {
    throw "cargo-packager completed without producing a MediaSift setup executable."
}

Write-Output "Windows installer: $($installer.FullName)"
