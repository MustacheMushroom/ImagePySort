# Imported utility-script inventory

The `extra_scripts/` directory preserves the original utilities. This document
records their behavior and the deliberate MediaSift integration boundary.

## `Rename-FilesWithCreationDate.ps1`

- **Behavior:** Recursively prefixes all filenames with a filesystem creation
  date, optionally using the older modification date. It avoids collisions and
  supports PowerShell `-WhatIf`.
- **MediaSift disposition:** Integrated as **Prefix media filenames by date**.
  MediaSift limits the operation to known media formats, skips existing
  `YYYY-MM-DD` plus `-` prefixes, resolves collisions, and requires an in-app
  confirmation.

## `photo_restoration_utility.py`

- **Behavior:** Uses Pillow for a mild median-filter, contrast, color, and
  sharpness enhancement; creates a copy with an `(Enhanced)` filename suffix;
  and prints a conservative external-AI restoration prompt.
- **MediaSift disposition:** Integrated as **Conservatively enhance photo** and
  **Copy AI restoration prompt**. The enhancement writes a PNG, BMP, or TIFF
  sibling copy and never changes the source. Prompt generation is explicitly
  for an external AI service; MediaSift does not claim to perform AI
  colorization. The original Pillow utility remains available for its wider
  format support.

## `Organize-DuplicateFiles.ps1`

- **Behavior:** Groups arbitrary files by size and SHA-256, then automatically
  moves one copy to `Originals` and others into grouped `Duplicates` folders. It
  can also move unique files.
- **MediaSift disposition:** Not ported. MediaSift already provides media-only
  SHA-256 review with a user-selected keeper per group and explicit recycle,
  deletion, or ZIP-backup actions. Auto-moving originals or unique files would
  be less safe.

## `Rename-OpenXMLZips.ps1`

- **Behavior:** Identifies document packages stored as ZIP archives and
  preserves their original `.zip` under `originals/` while creating a correctly
  extended document copy.
- **MediaSift disposition:** Not ported. This is document/package recovery rather
  than media management, so it remains a standalone utility to avoid broadening
  MediaSift's scope.

## Safety decisions

- The imported scripts are retained for their standalone workflows. Maintenance
  changes are limited to compatibility, logging, and static-analysis hygiene.
- The two logging utilities share `MediaSift.Logging.ps1`; keep that helper next
  to the scripts when copying either workflow elsewhere.
- MediaSift scans and renames only known image, video, and audio formats.
- Photo enhancement always creates a new file. Filename prefixing is confirmed
  in the UI before any rename occurs.
