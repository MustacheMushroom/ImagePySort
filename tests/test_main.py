"""Unit tests for main.py image-sorting logic."""
import os
import shutil
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from PIL import Image

from main import (
    EXIF_DATE_TAG,
    MONTH_NAMES,
    IMAGE_EXTENSIONS,
    file_hash,
    find_exact_duplicates,
    get_date_taken,
    iter_image_paths,
    select_directory,
    sort_images,
)


def _make_jpeg_with_exif(path: str, date_str: str) -> None:
    """Create a minimal JPEG at *path* with DateTimeOriginal set to *date_str*.

    date_str format: 'YYYY:MM:DD HH:MM:SS'
    """
    img = Image.new("RGB", (8, 8), color=(128, 128, 128))
    exif = img.getexif()
    exif[EXIF_DATE_TAG] = date_str
    img.save(path, format="JPEG", exif=exif.tobytes())


def _make_jpeg_no_exif(path: str) -> None:
    """Create a minimal JPEG at *path* with no EXIF data."""
    img = Image.new("RGB", (8, 8))
    img.save(path, format="JPEG")


class TestMonthNames(unittest.TestCase):
    def test_all_months_present(self):
        self.assertEqual(len(MONTH_NAMES), 12)

    def test_january(self):
        self.assertEqual(MONTH_NAMES["01"], "Jan")

    def test_december(self):
        self.assertEqual(MONTH_NAMES["12"], "Dec")

    def test_invalid_month_returns_none(self):
        self.assertIsNone(MONTH_NAMES.get("13"))


class TestGetDateTaken(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmpdir)

    def test_returns_date_for_image_with_exif(self):
        path = os.path.join(self.tmpdir, "photo.jpg")
        _make_jpeg_with_exif(path, "2023:06:15 10:30:00")
        result = get_date_taken(path)
        self.assertIsNotNone(result)
        self.assertTrue(result.startswith("2023"))

    def test_returns_none_for_image_without_exif(self):
        path = os.path.join(self.tmpdir, "plain.jpg")
        _make_jpeg_no_exif(path)
        self.assertIsNone(get_date_taken(path))

    def test_returns_none_for_nonexistent_file(self):
        self.assertIsNone(get_date_taken("/nonexistent/path/image.jpg"))

    def test_returns_none_for_non_image_file(self):
        path = os.path.join(self.tmpdir, "text.txt")
        with open(path, "w") as f:
            f.write("not an image")
        self.assertIsNone(get_date_taken(path))


class TestSortImages(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmpdir)

    def test_sorts_single_image_into_correct_folder(self):
        src = os.path.join(self.tmpdir, "img.jpg")
        _make_jpeg_with_exif(src, "2022:03:10 08:00:00")

        moved = sort_images(self.tmpdir)

        self.assertEqual(moved, 1)
        dest = os.path.join(self.tmpdir, "2022", "Mar", "img.jpg")
        self.assertTrue(os.path.isfile(dest), f"Expected file at {dest}")

    def test_skips_image_without_exif(self):
        src = os.path.join(self.tmpdir, "noexif.jpg")
        _make_jpeg_no_exif(src)

        moved = sort_images(self.tmpdir)

        self.assertEqual(moved, 0)
        # File should remain where it was
        self.assertTrue(os.path.isfile(src))

    def test_skips_non_image_files(self):
        path = os.path.join(self.tmpdir, "readme.txt")
        with open(path, "w") as f:
            f.write("hello")

        moved = sort_images(self.tmpdir)

        self.assertEqual(moved, 0)
        self.assertTrue(os.path.isfile(path))

    def test_creates_year_month_folders(self):
        src = os.path.join(self.tmpdir, "photo.jpg")
        _make_jpeg_with_exif(src, "2021:11:05 12:00:00")
        sort_images(self.tmpdir)

        self.assertTrue(os.path.isdir(os.path.join(self.tmpdir, "2021", "Nov")))

    def test_returns_count_of_moved_files(self):
        for i, date in enumerate(["2020:01:01 00:00:00", "2020:02:01 00:00:00"]):
            _make_jpeg_with_exif(os.path.join(self.tmpdir, f"photo{i}.jpg"), date)

        moved = sort_images(self.tmpdir)
        self.assertEqual(moved, 2)

    def test_image_extensions_recognised(self):
        expected = {'.png', '.jpg', '.jpeg', '.gif', '.bmp'}
        self.assertEqual(set(IMAGE_EXTENSIONS), expected)

    def test_second_run_is_idempotent(self):
        """Running sort_images twice should not move already-sorted files."""
        src = os.path.join(self.tmpdir, "photo.jpg")
        _make_jpeg_with_exif(src, "2023:07:20 09:00:00")

        first = sort_images(self.tmpdir)
        second = sort_images(self.tmpdir)

        self.assertEqual(first, 1)
        self.assertEqual(second, 0)
        dest = os.path.join(self.tmpdir, "2023", "Jul", "photo.jpg")
        self.assertTrue(os.path.isfile(dest))

    def test_skips_image_with_malformed_exif_date(self):
        """An image whose EXIF date is too short or wrongly formatted is skipped."""
        path = os.path.join(self.tmpdir, "bad_date.jpg")
        _make_jpeg_with_exif(path, "202")  # malformed — not 'YYYY:MM:DD HH:MM:SS'

        moved = sort_images(self.tmpdir)

        self.assertEqual(moved, 0)
        self.assertTrue(os.path.isfile(path))

    def test_filename_collision_renames_duplicate(self):
        """Two images with the same filename sorted to the same folder get distinct names."""
        dir_a = os.path.join(self.tmpdir, "a")
        dir_b = os.path.join(self.tmpdir, "b")
        os.makedirs(dir_a, exist_ok=True)
        os.makedirs(dir_b, exist_ok=True)

        date_str = "2024:01:02 03:04:05"
        _make_jpeg_with_exif(os.path.join(dir_a, "photo.jpg"), date_str)
        _make_jpeg_with_exif(os.path.join(dir_b, "photo.jpg"), date_str)

        moved = sort_images(self.tmpdir)

        self.assertEqual(moved, 2)
        dest_folder = os.path.join(self.tmpdir, "2024", "Jan")
        self.assertTrue(os.path.isfile(os.path.join(dest_folder, "photo.jpg")))
        self.assertTrue(os.path.isfile(os.path.join(dest_folder, "photo_1.jpg")))


class TestDuplicateScanning(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmpdir)

    def test_iter_image_paths_finds_images_recursively(self):
        nested = os.path.join(self.tmpdir, "nested")
        os.makedirs(nested)
        _make_jpeg_no_exif(os.path.join(self.tmpdir, "first.jpg"))
        _make_jpeg_no_exif(os.path.join(nested, "second.JPG"))
        with open(os.path.join(nested, "notes.txt"), "w") as file:
            file.write("not an image")

        paths = list(iter_image_paths(self.tmpdir))

        self.assertEqual(len(paths), 2)
        self.assertTrue(all(path.lower().endswith(".jpg") for path in paths))

    def test_iter_image_paths_rejects_missing_directory(self):
        with self.assertRaises(ValueError):
            list(iter_image_paths(os.path.join(self.tmpdir, "missing")))

    def test_file_hash_is_based_on_file_contents(self):
        first = os.path.join(self.tmpdir, "first.jpg")
        second = os.path.join(self.tmpdir, "second.jpg")
        _make_jpeg_no_exif(first)
        shutil.copyfile(first, second)

        self.assertEqual(file_hash(first), file_hash(second))

    def test_finds_identical_images_with_different_names_and_folders(self):
        folder = os.path.join(self.tmpdir, "nested")
        os.makedirs(folder)
        original = os.path.join(self.tmpdir, "original.jpg")
        copy = os.path.join(folder, "copy.jpg")
        _make_jpeg_no_exif(original)
        shutil.copyfile(original, copy)

        duplicates = find_exact_duplicates(self.tmpdir)

        self.assertEqual(len(duplicates), 1)
        self.assertEqual(next(iter(duplicates.values())), sorted([original, copy]))

    def test_does_not_report_different_images_as_duplicates(self):
        _make_jpeg_no_exif(os.path.join(self.tmpdir, "first.jpg"))
        image = Image.new("RGB", (8, 8), color=(255, 0, 0))
        image.save(os.path.join(self.tmpdir, "second.jpg"), format="JPEG")

        self.assertEqual(find_exact_duplicates(self.tmpdir), {})


class TestSelectDirectory(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()
        mock_tkinter = MagicMock()
        self._tk_patcher = patch.dict(
            sys.modules,
            {
                "tkinter": mock_tkinter,
                "tkinter.filedialog": mock_tkinter.filedialog,
                "tkinter.messagebox": mock_tkinter.messagebox,
            },
        )
        self._tk_patcher.start()
        self.mock_filedialog = mock_tkinter.filedialog
        self.mock_messagebox = mock_tkinter.messagebox

    def tearDown(self):
        self._tk_patcher.stop()
        shutil.rmtree(self.tmpdir)

    def test_sorts_and_shows_success_message(self):
        self.mock_filedialog.askdirectory.return_value = self.tmpdir
        with patch("main.sort_images", return_value=3) as mock_sort:
            select_directory()
            mock_sort.assert_called_once_with(self.tmpdir)
            self.mock_messagebox.showinfo.assert_called_once()

    def test_does_nothing_when_no_directory_selected(self):
        self.mock_filedialog.askdirectory.return_value = ""
        with patch("main.sort_images") as mock_sort:
            select_directory()
            mock_sort.assert_not_called()

    def test_shows_error_dialog_on_exception(self):
        self.mock_filedialog.askdirectory.return_value = self.tmpdir
        with patch("main.sort_images", side_effect=OSError("disk full")):
            select_directory()
            self.mock_messagebox.showerror.assert_called_once()


if __name__ == "__main__":
    unittest.main()
