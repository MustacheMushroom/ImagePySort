import calendar
import os
import shutil
from typing import Optional

from PIL import Image

# EXIF tag ID for "DateTimeOriginal" (date/time the photo was taken).
EXIF_DATE_TAG = 36867

# Mapping from zero-padded month number to abbreviated month name.
MONTH_NAMES: dict[str, str] = {
    str(i).zfill(2): calendar.month_abbr[i] for i in range(1, 13)
}

# Image file extensions that will be processed.
IMAGE_EXTENSIONS = ('.png', '.jpg', '.jpeg', '.gif', '.bmp')


def get_date_taken(path: str) -> Optional[str]:
    """Return the 'DateTimeOriginal' EXIF string from *path*, or None."""
    try:
        exif = Image.open(path).getexif()
        return exif.get(EXIF_DATE_TAG)
    except Exception:
        return None


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

            year, month = date[:4], date[5:7]
            month_name = MONTH_NAMES.get(month)
            if not month_name:
                continue

            dest_folder = os.path.join(directory, year, month_name)
            os.makedirs(dest_folder, exist_ok=True)

            dest_path = os.path.join(dest_folder, file)
            if os.path.abspath(file_path) == os.path.abspath(dest_path):
                continue

            shutil.move(file_path, dest_path)
            moved += 1

    return moved


def select_directory() -> None:
    """Open a directory chooser, sort images, then show a result dialog."""
    from tkinter import filedialog, messagebox  # noqa: PLC0415

    directory = filedialog.askdirectory()
    if directory:
        moved = sort_images(directory)
        messagebox.showinfo("Info", f"Done – {moved} image(s) sorted successfully!")


if __name__ == "__main__":
    import tkinter as tk

    app = tk.Tk()
    app.title("Image Sorter")

    frame = tk.Frame(app, padx=20, pady=20)
    frame.pack(padx=10, pady=10)

    label = tk.Label(frame, text="Select a directory to sort its images by date:")
    label.pack(pady=10)

    btn = tk.Button(frame, text="Select Directory", command=select_directory)
    btn.pack(pady=10)

    app.mainloop()
