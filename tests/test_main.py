"""Unit tests for main.py image-sorting logic."""
import os
import shutil
import tempfile
import unittest

from PIL import Image

from main import EXIF_DATE_TAG, MONTH_NAMES, IMAGE_EXTENSIONS, get_date_taken, sort_images


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


if __name__ == "__main__":
    unittest.main()
