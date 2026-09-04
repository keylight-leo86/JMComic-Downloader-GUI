# -*- coding: utf-8 -*-
"""JMComic Downloader GUI · 在线预览后端门面（只读，不落盘到下载目录）

职责：把 JMComic-Crawler-Python 的“查询能力”以 JSON/字节形式提供给前端：
  - search  关键词搜索（含车号直达）
  - album   本子详情 + 章节目录
  - photo   章节详情 + 图片清单
  - cover   封面图（原始字节，无需解密）
  - image   章节内单张图片（必要时做 scramble 解密，纯内存，不写盘）

设计要点：
  1. 线程模型 —— 本地 ThreadingHTTPServer 每请求一线程；JM client 非线程安全，
     因此用 threading.local 按线程懒建 JmOption + client。
  2. 设置热更新 —— 客户端签名 (impl, proxy) 取自 settings provider；某线程检测到
     签名变化会丢弃旧 client 重建，因此修改代理/客户端类型无需重启。
  3. 缓存 —— album/photo/cover/image 均为有界 LRU，避免翻页/重复浏览反复打上游。
     全部缓存内存态，绝无磁盘写入。
  4. 惰性导入 —— jmcomic 体积较大，推迟到第一次真实请求时才 import，保证
     GUI 启动/健康检查不被拖慢。

任何失败抛 PreviewError(code, message)，由 HTTP 层映射为 JSON 错误响应。
"""
from __future__ import annotations

import threading
from collections import OrderedDict
from io import BytesIO
from typing import Callable

# 客户端 impl 归一化（兼容旧版 settings 的中文标签与新版直接存 'api'/'html'）
_IMPL_ALIASES = {
    "移动端 API（推荐）": "api",
    "api": "api",
    "网页端（不稳定）": "html",
    "html": "html",
}
_DEFAULT_IMPL = "api"
# 旧版 settings.json 中 proxy 字段的两种中文标签
_PROXY_SYSTEM_LABEL = "跟随系统"
_PROXY_NONE_LABEL = "不使用代理"
# 封面尺寸白名单：'' 原图；'_3x4' 搜索列表缩略图；'_4x3' 横幅
_COVER_SIZES = ("", "_3x4", "_4x3")

_MIME_BY_SUFFIX = {
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".png": "image/png",
    ".webp": "image/webp",
    ".gif": "image/gif",
    ".avif": "image/avif",
}
_DEFAULT_IMAGE_MIME = "image/jpeg"


class PreviewError(Exception):
    """对 HTTP 层可见的预览错误。

    code  ∈ {"bad_request", "not_found", "network", "timeout", "internal"}
    status 对应 HTTP 状态码（由路由层使用）。
    """

    _STATUS = {
        "bad_request": 400,
        "not_found": 404,
        "network": 502,
        "timeout": 504,
        "internal": 500,
    }

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = self._STATUS.get(code, 500)


class _LRU:
    """线程安全的有界 LRU。get 不命中返回 None。"""

    def __init__(self, maxsize: int):
        self._maxsize = maxsize
        self._dict: "OrderedDict" = OrderedDict()
        self._lock = threading.RLock()

    def get(self, key):
        with self._lock:
            value = self._dict.get(key, None)
            if value is not None:
                self._dict.move_to_end(key)
            return value

    def put(self, key, value) -> None:
        with self._lock:
            self._dict[key] = value
            self._dict.move_to_end(key)
            while len(self._dict) > self._maxsize:
                self._dict.popitem(last=False)

    def clear(self) -> None:
        with self._lock:
            self._dict.clear()

    def __len__(self) -> int:
        with self._lock:
            return len(self._dict)


def _normalize_impl(raw) -> str:
    if isinstance(raw, str):
        impl = _IMPL_ALIASES.get(raw.strip())
        if impl:
            return impl
    return _DEFAULT_IMPL


def _fmt_proxies(proxy: str):
    """把旧/新版设置的 proxy 值归一为 jmcomic postman 接受的格式。

    返回 None = 跟随系统代理；{} = 显式不使用代理；
    {"http":..., "https":...} = 使用指定代理。
    """
    proxy = (proxy or "").strip()
    if proxy in ("", "system", _PROXY_SYSTEM_LABEL):
        return None
    if proxy in ("none", _PROXY_NONE_LABEL):
        return {}
    if not (proxy.startswith("http://") or proxy.startswith("https://")):
        proxy = f"http://{proxy}"
    return {"http": proxy, "https": proxy}


# --------------------------------------------------------------------------- #
# 图片 scramble 解密（对齐 JmImageTool.decode_and_save，纯内存实现）
# --------------------------------------------------------------------------- #
def decode_scramble_bytes(content: bytes, num: int) -> tuple:
    """按禁漫分割数 num 对图片字节做纵向重排解密。

    返回 (bytes, mime)：
      - num <= 0 或内容为空 → 原样返回，mime 由调用方按原始后缀判定（这里给 None）；
      - 否则用 PIL 按 num 段纵向重排（与库 decode_and_save 同一算法），
        并对 WEBP/PNG 用无损编码、JPEG 用高质量编码，避免预览二次劣化。
    """
    if num <= 0 or not content:
        return content, None

    from PIL import Image  # noqa: PLC0415 —— 惰性，首次图片请求才加载

    img = Image.open(BytesIO(content))
    fmt = (img.format or "JPEG").upper()
    if fmt == "JPG":
        fmt = "JPEG"
    if fmt == "WEBP":
        mime = "image/webp"
    elif fmt == "PNG":
        mime = "image/png"
    else:
        fmt = "JPEG"
        mime = "image/jpeg"

    width, height = img.size
    decoded = Image.new("RGB", (width, height))
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
        region = img.crop((0, y_src, width, y_src + move))
        decoded.paste(region, (0, y_dst))

    buffer = BytesIO()
    save_kwargs: dict = {}
    if fmt == "WEBP":
        save_kwargs = {"lossless": True}
    elif fmt == "JPEG":
        save_kwargs = {"quality": 95, "subsampling": 0}
    decoded.save(buffer, format=fmt, **save_kwargs)
    return buffer.getvalue(), mime


def _image_mime(suffix: str) -> str:
    return _MIME_BY_SUFFIX.get((suffix or "").lower(), _DEFAULT_IMAGE_MIME)


# --------------------------------------------------------------------------- #
# PreviewService
# --------------------------------------------------------------------------- #
class PreviewService:
    """无状态门面（缓存为实例内部有界 LRU）。settings_provider 返回当前设置 dict。"""

    # 缓存上限：图片字节约 100-500KB/张，64 张 ≈ ≤32MB；封面更小，多放一些
    _CACHE_LIMITS = {
        "album": 200,
        "photo": 120,
        "cover": 240,
        "image": 64,
    }

    def __init__(self, settings_provider: Callable[[], dict]):
        self._provider = settings_provider
        self._local = threading.local()
        self._album_cache = _LRU(self._CACHE_LIMITS["album"])
        self._photo_cache = _LRU(self._CACHE_LIMITS["photo"])
        self._cover_cache = _LRU(self._CACHE_LIMITS["cover"])
        self._image_cache = _LRU(self._CACHE_LIMITS["image"])
        # JmPhotoDetail 实体缓存：photo() 序列化与 image() 字节路由共用，
        # 避免首次浏览某章节时对同一详情发起两次上游请求
        self._entity_cache = _LRU(self._CACHE_LIMITS["photo"])

    # ---- client 生命周期 -------------------------------------------------- #
    def _signature(self) -> tuple:
        settings = self._provider() or {}
        return (_normalize_impl(settings.get("client")), (settings.get("proxy") or "").strip())

    def _client(self):
        """线程局部 client，签名变化时重建。jmcomic 首次使用才 import。"""
        signature = self._signature()
        cached = getattr(self._local, "client", None)
        cached_sig = getattr(self._local, "signature", None)
        if cached is not None and cached_sig == signature:
            return cached

        import jmcomic

        impl, proxy = signature
        client_cfg = {"impl": impl}
        proxies = _fmt_proxies(proxy)
        if proxies is not None:  # None = 跟随系统，不覆盖默认；{} = 显式禁用
            client_cfg["postman"] = {"meta_data": {"proxies": proxies}}

        option = jmcomic.JmOption.construct(
            {"client": client_cfg, "log": False},
        )
        client = option.build_jm_client()
        self._local.client = client
        self._local.signature = signature
        return client

    # ---- 查询能力 --------------------------------------------------------- #
    def search(self, query: str, page: int = 1) -> dict:
        query = (query or "").strip()
        if not query:
            raise PreviewError("bad_request", "请输入搜索关键词或车号")
        page = max(1, page)

        from jmcomic.jm_config import JmMagicConstants

        client = self._client()
        try:
            search_page = client.search(
                query, page, 0,
                "", "", JmMagicConstants.CATEGORY_ALL, None,
            )
        except PreviewError:
            raise
        except Exception as exc:  # noqa: BLE001 —— 网络/解析失败统一归类
            self._raise_network(exc)

        items = []
        for aid, info in search_page.content:
            items.append({
                "id": str(aid),
                "title": str(info.get("name") or "").strip(),
                "author": _pick_text(info.get("author")),
                "description": str(info.get("description") or "").strip(),
                "tags": _pick_str_list(info.get("tags")),
            })

        return {
            "query": query,
            "page": page,
            "pageSize": int(getattr(search_page, "page_size", 0) or 20),
            "total": int(getattr(search_page, "total", 0) or 0),
            "items": items,
        }

    def album(self, album_id) -> dict:
        album_id = self._require_id(album_id)
        cached = self._album_cache.get(album_id)
        if cached is not None:
            return cached

        client = self._client()
        try:
            album = client.get_album_detail(album_id)
        except Exception as exc:  # noqa: BLE001
            self._raise_network(exc, album_id)

        chapters = [
            {"id": str(pid), "index": int(pindex), "title": str(ptitle).strip()}
            for pid, pindex, ptitle in album.episode_list
        ]
        data = {
            "id": album.album_id,
            "title": album.name,
            "authors": list(album.authors or []),
            "tags": list(album.tags or []),
            "description": album.description or "",
            "pageCount": int(album.page_count),
            "views": str(album.views or ""),
            "likes": str(album.likes or ""),
            "commentCount": int(album.comment_count or 0),
            "pubDate": str(album.pub_date or ""),
            "updateDate": str(album.update_date or ""),
            "works": list(album.works or []),
            "actors": list(album.actors or []),
            "chapterCount": len(chapters),
            "chapters": chapters,
        }
        self._album_cache.put(album_id, data)
        return data

    def _photo_entity(self, photo_id: str):
        """取完整的 JmPhotoDetail（含 page_arr/scramble_id），带共享 LRU。"""
        cached = self._entity_cache.get(photo_id)
        if cached is not None:
            return cached

        client = self._client()
        try:
            # fetch_album=False：章节来自相册目录时 album 已由详情页取过；
            # fetch_scramble_id=True：图片字节路由需要 scramble_id 才能解密。
            photo = client.get_photo_detail(photo_id, fetch_album=False, fetch_scramble_id=True)
        except Exception as exc:  # noqa: BLE001
            self._raise_network(exc, photo_id)

        self._entity_cache.put(photo_id, photo)
        return photo

    def photo(self, photo_id) -> dict:
        photo_id = self._require_id(photo_id)
        cached = self._photo_cache.get(photo_id)
        if cached is not None:
            return cached

        photo = self._photo_entity(photo_id)

        page_count = len(photo)
        data = {
            "id": photo.photo_id,
            "albumId": str(photo.album_id or ""),
            "index": int(photo.album_index or 1),
            "title": photo.name or "",
            "isSingle": bool(photo.is_single_album),
            "author": photo.author or "",
            "tags": list(photo.tags or []),
            "pageCount": page_count,
            "images": [
                {"index": i + 1, "url": f"/api/image?photo={photo.photo_id}&index={i + 1}"}
                for i in range(page_count)
            ],
        }
        self._photo_cache.put(photo_id, data)
        return data

    def cover(self, album_id, size: str = "") -> tuple:
        """封面原始字节 → (bytes, mime)。size ∈ _COVER_SIZES。"""
        album_id = self._require_id(album_id)
        size = size if size in _COVER_SIZES else ""
        key = (album_id, size)

        cached = self._cover_cache.get(key)
        if cached is not None:
            return cached

        from jmcomic.jm_toolkit import JmcomicText

        url = JmcomicText.get_album_cover_url(album_id, size=size)
        client = self._client()
        try:
            resp = client.get_jm_image(url)
            resp.require_success()
        except Exception as exc:  # noqa: BLE001
            self._raise_network(exc, album_id)

        content = resp.content
        mime = "image/jpeg" if content else _DEFAULT_IMAGE_MIME
        self._cover_cache.put(key, (content, mime))
        return content, mime

    def image(self, photo_id, index: int) -> tuple:
        """章节内单张图片（必要时解密）→ (bytes, mime)。"""
        photo = self.photo(photo_id)  # 命中门面 photo 缓存
        if not 1 <= index <= photo["pageCount"]:
            raise PreviewError("bad_request",
                               f"图片序号越界：章节 {photo_id} 共 {photo['pageCount']} 页，请求第 {index} 页")

        cache_key = (photo_id, index)
        cached = self._image_cache.get(cache_key)
        if cached is not None:
            return cached

        client = self._client()
        try:
            # 实体级缓存保证 photo() 已取过详情时这里零额外请求
            detail = self._photo_entity(photo_id)
            image = detail[index - 1]  # 0 基索引
            resp = client.get_jm_image(image.download_url)
            resp.require_success()
        except Exception as exc:  # noqa: BLE001
            self._raise_network(exc, photo_id)

        content = resp.content
        suffix = (image.img_file_suffix or "").lower()
        mime = _image_mime(suffix)

        if suffix != ".gif":
            from jmcomic.jm_toolkit import JmImageTool
            try:
                num = JmImageTool.get_num_by_url(image.scramble_id, image.download_url)
            except Exception:  # noqa: BLE001 —— 解析失败视为无需解密
                num = 0
            if num > 0:
                content, decoded_mime = decode_scramble_bytes(content, num)
                mime = decoded_mime or mime

        self._image_cache.put(cache_key, (content, mime))
        return content, mime

    def status(self) -> dict:
        """轻量状态：当前生效的客户端配置与缓存占用（不触发网络请求）。"""
        impl, proxy = self._signature()
        return {
            "ok": True,
            "impl": impl,
            "proxy": proxy,
            "caches": {
                "album": len(self._album_cache),
                "photo": len(self._photo_cache),
                "entity": len(self._entity_cache),
                "cover": len(self._cover_cache),
                "image": len(self._image_cache),
            },
        }

    def clear_caches(self) -> None:
        for cache in (self._album_cache, self._photo_cache, self._cover_cache,
                      self._image_cache, self._entity_cache):
            cache.clear()

    # ---- helpers ---------------------------------------------------------- #
    @staticmethod
    def _require_id(value) -> str:
        text = str(value or "").strip()
        if not text or not text.isdigit():
            raise PreviewError("bad_request", f"无效的车号/ID：{text!r}")
        return text

    @staticmethod
    def _raise_network(exc: Exception, jm_id: str = "") -> None:
        """把库异常归一为可读的 PreviewError。"""
        name = type(exc).__name__
        text = str(exc)
        lower = text.lower()
        scope = f"（{jm_id}）" if jm_id else ""
        if any(token in lower for token in ("timeout", "timed out", "timedout")):
            raise PreviewError("timeout", f"请求超时{scope}，请检查网络或代理设置") from exc
        if any(token in lower for token in ("connect", "connection", "refused", "dns", "resolve")):
            raise PreviewError("network", f"网络连接失败{scope}：{name}，请检查网络或代理设置") from exc
        if any(token in lower for token in ("not found", "404", "missing", "缺失")):
            raise PreviewError("not_found", f"未找到该作品{scope}，请确认车号/ID 是否正确") from exc
        if any(token in lower for token in ("ssl", "certificate", "handshake")):
            raise PreviewError("network", f"SSL/TLS 握手失败{scope}，站点可能要求更换镜像或代理") from exc
        # 其他（含禁漫风控/响应为空）
        raise PreviewError("network", f"上游请求失败{scope}：{name} {text[:120]}") from exc


# 便捷：供测试与路由层使用的纯函数
def _pick_text(value) -> str:
    if isinstance(value, (list, tuple)):
        value = value[0] if value else ""
    return str(value or "").strip()


def _pick_str_list(value) -> list:
    if isinstance(value, (list, tuple)):
        return [str(item) for item in value]
    if isinstance(value, str) and value.strip():
        return [item for item in (v.strip() for v in value.replace("，", ",").split(",")) if item]
    return []
