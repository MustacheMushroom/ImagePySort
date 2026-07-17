# ImagePySort
A desktop utility to organize images in folders based on the date taken and to
find exact duplicate images.

Run `python main.py`, then choose either:

- **Select Directory** to organize photos into `Year/Month` folders using EXIF
  date information.
- **Scan for Exact Duplicates** to select a folder or drive root (for example,
  `D:\\`). Duplicate images are identified by their SHA-256 file-content hash;
  visually similar images or differently encoded copies are not reported. The
  scan is read-only and does not delete or move any files.
