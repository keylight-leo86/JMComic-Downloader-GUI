import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from PIL import Image
from pypdf import PdfReader

from jmcomic_gui import export_result_pdfs, initial_output_directory, pdf_filename_for_photo


class FakeImage:
    def __init__(self, save_path):
        self.save_path = str(save_path)


class FakePhoto:
    def __init__(self, album, index, name, image_paths):
        self.from_album = album
        self.index = index
        self.album_index = index
        self.name = name
        self._images = [FakeImage(path) for path in image_paths]

    def __iter__(self):
        return iter(self._images)

    def __len__(self):
        return len(self._images)

    def is_album(self):
        return False


class FakeAlbum:
    def __init__(self, album_id, name, photos):
        self.id = album_id
        self.album_id = album_id
        self.name = name
        self._photos = photos

    def __iter__(self):
        return iter(self._photos)

    def __len__(self):
        return len(self._photos)

    def is_album(self):
        return True


class FakeOption:
    @staticmethod
    def decide_image_filepath(image):
        return image.save_path


class TestGuiDefaultOutput(unittest.TestCase):
    def test_default_output_follows_current_application_directory(self):
        with patch("jmcomic_gui.app_directory", return_value=Path("C:/Apps/JMComic-Downloader-GUI")):
            self.assertEqual(
                initial_output_directory({}),
                Path("C:/Apps/JMComic-Downloader-GUI/下载"),
            )

    def test_saved_default_moves_with_application(self):
        settings = {"output_mode": "default", "output": "D:/Old/下载"}
        with patch("jmcomic_gui.app_directory", return_value=Path("E:/New/JMComic-Downloader-GUI")):
            self.assertEqual(
                initial_output_directory(settings),
                Path("E:/New/JMComic-Downloader-GUI/下载"),
            )

    def test_custom_output_is_preserved(self):
        settings = {"output_mode": "custom", "output": "D:/Comics"}
        with patch("jmcomic_gui.app_directory", return_value=Path("E:/App")):
            self.assertEqual(initial_output_directory(settings), Path("D:/Comics"))

    def test_legacy_default_output_is_migrated(self):
        settings = {"output": "D:/OldPackage/下载"}
        with patch("jmcomic_gui.app_directory", return_value=Path("E:/NewPackage")):
            self.assertEqual(initial_output_directory(settings), Path("E:/NewPackage/下载"))


class TestGuiPdfExport(unittest.TestCase):
    def _make_pages(self, root, count, prefix):
        paths = []
        for index in range(count):
            path = root / f"{prefix}_{index}.png"
            Image.new("RGB", (80 + index, 100 + index), "white").save(path)
            paths.append(path)
        return paths

    def test_single_chapter_uses_album_name(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            album = FakeAlbum("100", "单章本子", [])
            photo = FakePhoto(album, 1, "单章本子", self._make_pages(root, 2, "single"))
            album._photos = [photo]
            result = SimpleNamespace(detail=album, downloader=object())

            exported, failures = export_result_pdfs(result, FakeOption(), root)

            self.assertFalse(failures)
            self.assertEqual([path.name for path in exported], ["单章本子.pdf"])
            self.assertEqual(len(PdfReader(str(exported[0])).pages), 2)
            self.assertFalse(any(Path(image.save_path).exists() for image in photo._images))

    def test_multi_chapter_adds_chapter_suffix(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            album = FakeAlbum("200", "多章本子", [])
            photos = [
                FakePhoto(album, 1, "开篇", self._make_pages(root, 1, "chapter1")),
                FakePhoto(album, 2, "终章", self._make_pages(root, 1, "chapter2")),
            ]
            album._photos = photos
            result = SimpleNamespace(detail=album, downloader=object())

            exported, failures = export_result_pdfs(result, FakeOption(), root)

            self.assertFalse(failures)
            self.assertEqual(
                [path.name for path in exported],
                ["多章本子 - 第1话 开篇.pdf", "多章本子 - 第2话 终章.pdf"],
            )
            for path in exported:
                self.assertEqual(len(PdfReader(str(path)).pages), 1)
            self.assertFalse(
                any(Path(image.save_path).exists() for photo in photos for image in photo._images)
            )

    def test_photo_download_uses_parent_album_name(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            album = FakeAlbum("300", "章节本子", [])
            photo = FakePhoto(album, 4, "第四话", self._make_pages(root, 1, "photo"))
            album._photos = [photo, object()]
            result = SimpleNamespace(detail=photo, downloader=object())

            exported, failures = export_result_pdfs(result, FakeOption(), root)

            self.assertFalse(failures)
            self.assertEqual([path.name for path in exported], ["章节本子 - 第4话 第四话.pdf"])
            self.assertEqual(pdf_filename_for_photo(album, photo), exported[0].name)
            self.assertFalse(any(Path(image.save_path).exists() for image in photo._images))

    def test_pdf_failure_keeps_source_images(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bad_image = root / "broken.webp"
            bad_image.write_bytes(b"not an image")
            album = FakeAlbum("400", "失败保留图片", [])
            photo = FakePhoto(album, 1, "失败保留图片", [bad_image])
            album._photos = [photo]
            result = SimpleNamespace(detail=album, downloader=object())

            exported, failures = export_result_pdfs(result, FakeOption(), root)

            self.assertFalse(exported)
            self.assertEqual(len(failures), 1)
            self.assertTrue(bad_image.exists())


if __name__ == "__main__":
    unittest.main()
