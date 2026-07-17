from __future__ import annotations

import calendar
import hashlib
import os
import shutil
from collections import defaultdict
from typing import Iterator, Optional

from PIL import Image

# EXIF tag ID for "DateTimeOriginal" (date/time the photo was taken).
EXIF_DATE_TAG = 36867

# Mapping from zero-padded month number to abbreviated month name.
MONTH_NAMES: dict[str, str] = {
    str(i).zfill(2): calendar.month_abbr[i] for i in range(1, 13)
}

# Image file extensions that will be processed.
IMAGE_EXTENSIONS = ('.png', '.jpg', '.jpeg', '.gif', '.bmp')

# Files are read in chunks so scanning large drives does not load whole images
# into memory.
HASH_CHUNK_SIZE = 1024 * 1024


def get_date_taken(path: str) -> Optional[str]:
    """Return the 'DateTimeOriginal' EXIF string from *path*, or None."""
    try:
        with Image.open(path) as image:
            return image.getexif().get(EXIF_DATE_TAG)
    except Exception:
        return None


def iter_image_paths(directory: str) -> Iterator[str]:
    """Yield absolute paths for supported images below *directory*.

    ``directory`` may be a normal folder or a drive root such as ``D:\\``.
    Symlinked directories are not followed, which prevents a scan from looping
    or unexpectedly leaving the selected drive.
    """
    if not os.path.isdir(directory):
        raise ValueError(f"Directory does not exist: {directory}")

    for root, directories, files in os.walk(directory, followlinks=False):
        # Keep results deterministic and avoid following directory symlinks.
        directories[:] = sorted(
            name for name in directories
            if not os.path.islink(os.path.join(root, name))
        )
        for file in sorted(files):
            if file.lower().endswith(IMAGE_EXTENSIONS):
                yield os.path.abspath(os.path.join(root, file))


def file_hash(path: str, chunk_size: int = HASH_CHUNK_SIZE) -> str:
    """Return the SHA-256 hash of a file's bytes."""
    digest = hashlib.sha256()
    with open(path, "rb") as image_file:
        for chunk in iter(lambda: image_file.read(chunk_size), b""):
            digest.update(chunk)
    return digest.hexdigest()


def find_exact_duplicates(directory: str) -> dict[str, list[str]]:
    """Find byte-for-byte duplicate images below *directory*.

    The returned dictionary maps a SHA-256 hash to the sorted absolute paths
    in each duplicate group.  Files with unique sizes are never hashed, which
    keeps drive scans efficient.  Unreadable files are skipped.
    """
    paths_by_size: dict[int, list[str]] = defaultdict(list)
    for path in iter_image_paths(directory):
        try:
            paths_by_size[os.path.getsize(path)].append(path)
        except OSError:
            # The file may have been removed or become inaccessible mid-scan.
            continue

    paths_by_hash: dict[str, list[str]] = defaultdict(list)
    for paths in paths_by_size.values():
        if len(paths) < 2:
            continue
        for path in paths:
            try:
                paths_by_hash[file_hash(path)].append(path)
            except OSError:
                continue

    return {
        digest: sorted(paths)
        for digest, paths in paths_by_hash.items()
        if len(paths) > 1
    }


def sort_images(directory: str) -> int:
    """Move images under *directory* into Year/Month sub-folders.

    Returns the number of image files that were successfully moved.
    """
    moved = 0
    for root, _, files in os.walk(directory):
        for file in files:
            if not file.lower().endswith(IMAGE_EXTENSIONS):
                continue

            file_path = os.path.join(root, file)
            date = get_date_taken(file_path)
            if not date:
                continue

            # Validate EXIF date string format before slicing.
            if (
                not isinstance(date, str)
                or len(date) < 7
                or date[4] != ":"
                or not date[:4].isdigit()
                or not date[5:7].isdigit()
            ):
                continue

            year, month = date[:4], date[5:7]
            month_name = MONTH_NAMES.get(month)
            if not month_name:
                continue

            dest_folder = os.path.join(directory, year, month_name)
            os.makedirs(dest_folder, exist_ok=True)

            dest_path = os.path.join(dest_folder, file)
            if os.path.abspath(file_path) == os.path.abspath(dest_path):
                continue

            # Avoid overwriting an existing file with the same name.
            base_name, ext = os.path.splitext(file)
            counter = 1
            while os.path.exists(dest_path):
                dest_path = os.path.join(dest_folder, f"{base_name}_{counter}{ext}")
                counter += 1

            shutil.move(file_path, dest_path)
            moved += 1

    return moved


def select_directory() -> None:
    """Open a directory chooser, sort images, then show a result dialog."""
    from tkinter import filedialog, messagebox  # noqa: PLC0415

    directory = filedialog.askdirectory()
    if directory:
        try:
            moved = sort_images(directory)
            messagebox.showinfo("Info", f"Done - {moved} image(s) sorted successfully!")
        except Exception as e:
            messagebox.showerror(
                "Error",
                f"An error occurred while sorting images:\n{e}",
            )


def _format_duplicate_report(duplicates: dict[str, list[str]]) -> str:
    """Format duplicate groups for display in the desktop interface."""
    if not duplicates:
        return "No exact duplicate images were found."

    extra_copies = sum(len(paths) - 1 for paths in duplicates.values())
    lines = [
        f"Found {len(duplicates)} duplicate group(s) "
        f"({extra_copies} extra copy/copies).",
        "",
    ]
    for index, paths in enumerate(duplicates.values(), start=1):
        lines.append(f"Group {index}:")
        lines.extend(f"  {path}" for path in paths)
        lines.append("")
    return "\n".join(lines)


def select_directory_for_duplicate_scan() -> None:
    """Choose a folder or drive, then show any exact duplicate images."""
    from tkinter import filedialog, messagebox  # noqa: PLC0415

    directory = filedialog.askdirectory(title="Select a folder or drive to scan")
    if not directory:
        return

    try:
        duplicates = find_exact_duplicates(directory)
    except (OSError, ValueError) as error:
        messagebox.showerror("Duplicate scan failed", str(error))
        return

    report = _format_duplicate_report(duplicates)
    if not duplicates:
        messagebox.showinfo("Exact duplicate scan", report)
        return

    # A dedicated, scrollable window is practical for reports from a full drive.
    import tkinter as tk  # noqa: PLC0415

    report_window = tk.Toplevel()
    report_window.title("Exact duplicate images")
    report_window.geometry("800x500")
    text = tk.Text(report_window, wrap="word")
    scrollbar = tk.Scrollbar(report_window, command=text.yview)
    text.configure(yscrollcommand=scrollbar.set)
    text.insert("1.0", report)
    text.configure(state="disabled")
    text.pack(side="left", fill="both", expand=True, padx=(10, 0), pady=10)
    scrollbar.pack(side="right", fill="y", padx=(0, 10), pady=10)


if __name__ == "__main__":
    import tkinter as tk

    app = tk.Tk()
    app.title("Image Sorter")

    frame = tk.Frame(app, padx=20, pady=20)
    frame.pack(padx=10, pady=10)

    label = tk.Label(frame, text="Organize images by date or scan for exact duplicates:")
    label.pack(pady=10)

    btn = tk.Button(frame, text="Select Directory", command=select_directory)
    btn.pack(pady=10)

    duplicate_btn = tk.Button(
        frame,
        text="Scan for Exact Duplicates",
        command=select_directory_for_duplicate_scan,
    )
    duplicate_btn.pack(pady=10)

    app.mainloop()
