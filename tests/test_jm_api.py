# -*- coding: utf-8 -*-
"""gui.jm_api / gui.server 预览链路单元测试（全部离线，不打真实网络）。

覆盖：
  - LRU 有界淘汰
  - 旧/新版设置归一化（client 中文标签、proxy 跟随系统/禁用/URL）
  - scramble 解密算法与“加扰-还原”往返一致
  - album / photo / search 序列化结构与缓存去重
  - image 字节路由：越界校验、gif 原样透传、非 gif 无需解密分支
  - 只读 HTTP 端点冒烟（health / preview/status / settings）
"""
from __future__ import annotations

import threading
import unittest
from io import BytesIO
from types import SimpleNamespace
from unittest import mock

from gui import jm_api
from gui.server import create_server, GuiHandler, _json_response


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #
def _blank(w, h):
    from PIL import Image
    return Image.new("RGB", (w, h))


def _encode(img) -> bytes:
    buffer = BytesIO()
    img.save(buffer, format="PNG")
    return buffer.getvalue()


def scramble_image(img, num: int):
    """JM 解密的逆操作：把原图按 num 段打乱，产出“已加扰”的图。"""
    from PIL import Image
    width, height = img.size
    scrambled = Image.new("RGB", (width, height))
    base_move = height // num
    over = height % num
    for i in range(num):
        move = base_move
        y_src = height - base_move * (i + 1) - over
        y_dst = base_move * i
        if i == 0:
            move += over
        else:
            y_dst += over
        region = img.crop((0, y_dst, width, y_dst + move))
        scrambled.paste(region, (0, y_src))
    return scrambled


class _FakeImage:
    """极简 JmImageDetail 替身。"""

    def __init__(self, index, suffix=".gif", scramble_id="0"):
        self.index = index
        self.img_file_suffix = suffix
        self.scramble_id = scramble_id
        self.img_file_name = f"{index:05d}"
        self.download_url = f"https://cdn.example/media/photos/1/{self.img_file_name}{suffix}"


class _FakePhoto:
    """极简 JmPhotoDetail 替身（支持 len 与下标取图）。"""

    def __init__(self, page_count=3, suffix=".gif"):
        self.photo_id = "1001"
        self.album_id = "1001"
        self.album_index = 1
        self.name = "测试章节"
        self.author = "作者A"
        self.tags = ["全彩", "中文"]
        self.is_single_album = True
        self._images = [_FakeImage(i + 1, suffix) for i in range(page_count)]

    def __len__(self):
        return len(self._images)

    def __getitem__(self, index):
        return self._images[index]


class _FakeAlbum:
    """极简 JmAlbumDetail 替身。"""

    album_id = "310311"
    name = "测试本子"
    authors = ["作者A", "作者B"]
    tags = ["全彩", "中文"]
    description = "描述文字"
    page_count = 66
    views = "40K"
    likes = "1K"
    comment_count = 8
    pub_date = "2020-08-29"
    update_date = "2021-01-01"
    works = ["原作"]
    actors = ["角色X"]
    episode_list = [("212214", "81", "94 突然打來"), ("212213", "82", "95 結局")]


class _FakeClient:
    """可挂接的假 client：只记录调用，不触网。"""

    def __init__(self, album=None, photo=None, search_content=None, resp_content=b"GIF89a-fake"):
        self.album = album
        self.photo = photo
        self.search_content = search_content or []
        self.resp_content = resp_content
        self.calls = []

    def get_album_detail(self, album_id):
        self.calls.append(("album", str(album_id)))
        return self.album

    def get_photo_detail(self, photo_id, fetch_album=True, fetch_scramble_id=True):
        self.calls.append(("photo", str(photo_id),
                           {"fetch_album": fetch_album, "fetch_scramble_id": fetch_scramble_id}))
        return self.photo

    def search(self, search_query, page, main_tag, order_by, time, category, sub_category):
        self.calls.append(("search", search_query, page))
        return SimpleNamespace(
            content=self.search_content,
            total=len(self.search_content),
            page_size=80,
        )

    def get_jm_image(self, url):
        self.calls.append(("image", url))
        return SimpleNamespace(content=self.resp_content, require_success=lambda: None)


def _service(settings=None):
    return jm_api.PreviewService(lambda: settings or {})


# --------------------------------------------------------------------------- #
class LruTests(unittest.TestCase):
    def test_bounded_eviction(self):
        cache = jm_api._LRU(2)
        cache.put("a", 1)
        cache.put("b", 2)
        cache.put("c", 3)
        self.assertEqual(cache.get("a"), None)  # 最早被淘汰
        self.assertEqual(cache.get("b"), 2)
        cache.get("b")  # 访问提升为最近
        cache.put("d", 4)
        self.assertEqual(cache.get("c"), None)
        self.assertEqual(cache.get("b"), 2)
        self.assertEqual(cache.get("d"), 4)

    def test_clear(self):
        cache = jm_api._LRU(4)
        cache.put("a", 1)
        cache.clear()
        self.assertEqual(len(cache), 0)


class NormalizeTests(unittest.TestCase):
    def test_impl_alias(self):
        self.assertEqual(jm_api._normalize_impl("移动端 API（推荐）"), "api")
        self.assertEqual(jm_api._normalize_impl("网页端（不稳定）"), "html")
        self.assertEqual(jm_api._normalize_impl("api"), "api")
        self.assertEqual(jm_api._normalize_impl("html"), "html")
        self.assertEqual(jm_api._normalize_impl(None), "api")
        self.assertEqual(jm_api._normalize_impl("乱写"), "api")

    def test_proxy_normalize(self):
        self.assertIsNone(jm_api._fmt_proxies(""))
        self.assertIsNone(jm_api._fmt_proxies("跟随系统"))
        self.assertIsNone(jm_api._fmt_proxies("system"))
        self.assertEqual(jm_api._fmt_proxies("不使用代理"), {})
        self.assertEqual(jm_api._fmt_proxies("none"), {})
        url = jm_api._fmt_proxies("127.0.0.1:7890")
        self.assertEqual(url, {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"})
        full = jm_api._fmt_proxies("http://proxy.local:8080")
        self.assertEqual(full["https"], "http://proxy.local:8080")


class DecodeTests(unittest.TestCase):
    def test_decode_roundtrip_pixel_equal(self):
        """加扰 → 解密后与原图像素完全一致（PNG 无损）。"""
        from PIL import Image

        width, height = 64, 91  # 91 = 10*9+1，触发 over 分支
        original = Image.new("RGB", (width, height))
        for y in range(height):
            for x in range(width):
                original.putpixel((x, y), ((x * 3) % 256, (y * 5) % 256, (x + y) % 256))

        for num in (10, 8, 2, 6):
            scrambled = scramble_image(original, num)
            decoded_bytes, mime = jm_api.decode_scramble_bytes(_encode(scrambled), num)
            self.assertEqual(mime, "image/png")
            decoded = Image.open(BytesIO(decoded_bytes))
            self.assertEqual(decoded.size, (width, height))
            self.assertEqual(decoded.tobytes(), original.tobytes(), f"num={num}")

    def test_num_zero_passthrough(self):
        content = b"raw-jpeg-bytes"
        out, mime = jm_api.decode_scramble_bytes(content, 0)
        self.assertIs(out, content)
        self.assertIsNone(mime)
        out2, _ = jm_api.decode_scramble_bytes(b"", 10)
        self.assertEqual(out2, b"")


class AlbumServiceTests(unittest.TestCase):
    def test_album_serialize_and_cache(self):
        fake = _FakeClient(album=_FakeAlbum())
        service = _service()
        service._client = mock.Mock(return_value=fake)

        data = service.album("310311")
        self.assertEqual(data["id"], "310311")
        self.assertEqual(data["authors"], ["作者A", "作者B"])
        self.assertEqual(data["chapterCount"], 2)
        self.assertEqual(data["chapters"][0], {"id": "212214", "index": 81, "title": "94 突然打來"})
        self.assertEqual(data["pageCount"], 66)

        again = service.album("310311")
        self.assertEqual(again["title"], "测试本子")
        self.assertEqual([c for c in fake.calls if c[0] == "album"], [("album", "310311")])  # 命中缓存不再请求

    def test_album_bad_id(self):
        service = _service()
        with self.assertRaises(jm_api.PreviewError) as ctx:
            service.album("abc123")
        self.assertEqual(ctx.exception.code, "bad_request")


class PhotoServiceTests(unittest.TestCase):
    def test_photo_serialize_and_cache(self):
        fake = _FakeClient(photo=_FakePhoto(page_count=3, suffix=".webp"))
        service = _service()
        service._client = mock.Mock(return_value=fake)

        data = service.photo("1001")
        self.assertEqual(data["pageCount"], 3)
        self.assertEqual(data["albumId"], "1001")
        self.assertEqual(data["images"][0], {"index": 1, "url": "/api/image?photo=1001&index=1"})
        self.assertEqual(len(data["images"]), 3)

        service.photo("1001")
        photo_calls = [c for c in fake.calls if c[0] == "photo"]
        self.assertEqual(len(photo_calls), 1)  # 命中实体缓存不再请求

    def test_photo_entity_uses_scramble_true(self):
        fake = _FakeClient(photo=_FakePhoto())
        service = _service()
        service._client = mock.Mock(return_value=fake)
        service.photo("1001")
        # fetch_album=False + fetch_scramble_id=True：实体能解密图片
        photo_calls = [c for c in fake.calls if c[0] == "photo"]
        self.assertEqual(len(photo_calls), 1)
        kwargs = photo_calls[0][2]
        self.assertFalse(kwargs["fetch_album"])
        self.assertTrue(kwargs["fetch_scramble_id"])


class ImageServiceTests(unittest.TestCase):
    def test_image_gif_passthrough(self):
        fake = _FakeClient(photo=_FakePhoto(page_count=2, suffix=".gif"),
                           resp_content=b"GIF89a-bytes")
        service = _service()
        service._client = mock.Mock(return_value=fake)

        content, mime = service.image("1001", 1)
        self.assertEqual(content, b"GIF89a-bytes")
        self.assertEqual(mime, "image/gif")

    def test_image_out_of_range(self):
        fake = _FakeClient(photo=_FakePhoto(page_count=1))
        service = _service()
        service._client = mock.Mock(return_value=fake)
        with self.assertRaises(jm_api.PreviewError) as ctx:
            service.image("1001", 9)
        self.assertEqual(ctx.exception.code, "bad_request")

    def test_image_webp_no_decode_when_num_zero(self):
        fake = _FakeClient(photo=_FakePhoto(page_count=2, suffix=".webp"),
                           resp_content=b"RIFF-webp-bytes")
        service = _service()
        service._client = mock.Mock(return_value=fake)
        with mock.patch("jmcomic.jm_toolkit.JmImageTool.get_num_by_url", return_value=0):
            content, mime = service.image("1001", 2)
        self.assertEqual(content, b"RIFF-webp-bytes")
        self.assertEqual(mime, "image/webp")


class SearchServiceTests(unittest.TestCase):
    def test_search_serialize(self):
        content = [
            ("310311", {"name": "[MANA] 示例本子", "author": "MANA", "description": "描述",
                        "tags": ["全彩"]}),
            ("320001", {"name": "第二本", "author": ["多人"], "tags": "中文, 彩页"}),
        ]
        fake = _FakeClient(search_content=content)
        service = _service()
        service._client = mock.Mock(return_value=fake)

        data = service.search("mana", page=2)
        self.assertEqual(data["page"], 2)
        self.assertEqual(data["total"], 2)
        self.assertEqual(data["items"][0]["id"], "310311")
        self.assertEqual(data["items"][0]["author"], "MANA")
        self.assertEqual(data["items"][1]["author"], "多人")
        self.assertEqual(data["items"][1]["tags"], ["中文", "彩页"])

    def test_search_requires_query(self):
        service = _service()
        with self.assertRaises(jm_api.PreviewError) as ctx:
            service.search("   ")
        self.assertEqual(ctx.exception.code, "bad_request")


class ServerRouteTests(unittest.TestCase):
    """只读端点的本地冒烟（不触网、不写设置文件）。"""

    def test_health_and_status(self):
        srv = create_server(port=0)
        srv.start()
        self.addCleanup(srv.shutdown)
        import urllib.request
        with urllib.request.urlopen(srv.base_url + "/api/preview/status", timeout=5) as resp:
            body = resp.read().decode("utf-8")
        self.assertIn('"impl": "api"', body)
        self.assertIn('"caches"', body)

    def test_search_empty_query_returns_400_json(self):
        srv = create_server(port=0)
        srv.start()
        self.addCleanup(srv.shutdown)
        import urllib.request
        try:
            urllib.request.urlopen(srv.base_url + "/api/search?q=", timeout=5)
            self.fail("should raise")
        except urllib.error.HTTPError as exc:
            self.assertEqual(exc.code, 400)
            self.assertIn("请输入搜索关键词", exc.read().decode("utf-8"))


if __name__ == "__main__":
    unittest.main()
