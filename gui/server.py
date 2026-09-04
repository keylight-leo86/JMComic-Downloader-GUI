# -*- coding: utf-8 -*-
"""JMComic Downloader GUI · 本地 HTTP 服务

职责：
- 托管 static/ 前端（pywebview WebView2 壳 / 浏览器调试同源加载）
- /api/health     健康检查
- /api/window/*   自绘标题栏按钮/拖动/边缘缩放 → 按 wid(main/reader) 直控窗口 HWND
                  （ctypes 消息跨线程安全；无壳返回 window not ready，--browser 天然兼容）
- /api/settings   设置读写（schema 兼容旧版 jmcomic_gui 的 settings.json，
                  含旧路径迁移与“默认输出目录跟随应用目录”逻辑）

架构对齐 Novelist：单进程 = 本服务线程 + pywebview(WebView2) 双窗口壳（主窗 + 独立
阅读窗，frameless + WndProc 子类化自绘标题栏，WebView2 独立 UserDataFolder 隔离）；
前端一切数据交互都走本服务的 HTTP API。
窗口控制经 ctypes PostMessage/ShowWindow/SetWindowPos 直控 HWND（跨线程安全，无需
GUI 线程 marshal）；desktop.py 在窗口出现后调用 set_main_hwnd/set_reader_hwnd 注册句柄。
"""
from __future__ import annotations

import ctypes
import json
import os
import queue
import subprocess
import sys
import threading
import time
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from . import jm_api
from . import jobs as gui_jobs

# --------------------------------------------------------------------------- #
# 窗口控制：双窗口 hwnd 直控（pywebview 壳在窗口出现后注册；ctypes 跨线程安全）
#   main   —— 主窗口（默认，无 wid 参数时命中）
#   reader —— 独立阅读窗口（详情页「在新窗口阅读」打开）
# --------------------------------------------------------------------------- #
_main_hwnd: list[int] = [0]
_reader_hwnd: list[int] = [0]


def set_main_hwnd(hwnd: int) -> None:
    _main_hwnd[0] = int(hwnd)


def get_main_hwnd() -> int:
    return _main_hwnd[0]


def set_reader_hwnd(hwnd: int) -> None:
    _reader_hwnd[0] = int(hwnd)


def get_reader_hwnd() -> int:
    return _reader_hwnd[0]


# --------------------------------------------------------------------------- #
# 设置持久化（与旧版 jmcomic_gui 行为一致）
# --------------------------------------------------------------------------- #
def _app_data_dir() -> Path:
    base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
    return Path(base) / "JMComic-Downloader-GUI"


def settings_path() -> Path:
    return _app_data_dir() / "settings.json"


def legacy_settings_path() -> Path:
    """旧版路径（v1.2.0 之前）：首次运行兼容读取。"""
    return Path(os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")) / "JMComicDownloader" / "settings.json"


def load_settings() -> dict:
    for path in (settings_path(), legacy_settings_path()):
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError, TypeError):
            continue
    return {}


def save_settings(data: dict) -> None:
    path = settings_path()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
    except OSError:
        pass


# 内存态设置：GET 直读、POST 落盘后刷新；预览门面通过 current_settings() 感知代理/客户端变更
_SETTINGS_CACHE: dict = {}


def _refresh_settings_cache() -> None:
    global _SETTINGS_CACHE
    _SETTINGS_CACHE = load_settings() or {}


def current_settings() -> dict:
    return _SETTINGS_CACHE


_refresh_settings_cache()


# --------------------------------------------------------------------------- #
# 主窗口句柄（app exe 目录/默认输出目录，用于前端“打开 PDF 目录”等逻辑）
# --------------------------------------------------------------------------- #
def app_directory() -> Path:
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).resolve().parent.parent


def default_download_directory() -> Path:
    return app_directory() / "下载"


def paths_equal(left: str | Path, right: str | Path) -> bool:
    try:
        return Path(left).resolve() == Path(right).resolve()
    except OSError:
        return str(left).strip() == str(right).strip()


def initial_output_directory(settings: dict) -> Path:
    """迁移兼容：自定义位置保留；默认/旧版默认位置始终跟随当前应用目录。"""
    if settings.get("output_mode") == "custom" and settings.get("output"):
        return Path(settings["output"])
    if settings.get("output_mode") == "default" or not settings:
        return default_download_directory()
    if not settings.get("output_mode") and settings.get("output"):
        # 旧版只有 output 字段：跟随应用目录迁移（不沿用旧绝对路径）
        return default_download_directory()
    return default_download_directory()


# --------------------------------------------------------------------------- #
# /api/window/* 路由
# --------------------------------------------------------------------------- #
def _post(hwnd: int, msg: int, wparam: int, lparam: int = 0) -> None:
    ctypes.windll.user32.PostMessageW(hwnd, msg, wparam, lparam)


WM_SYSCOMMAND = 0x0112
SC_MINIMIZE = 0xF020
SC_MAXIMIZE = 0xF030
SC_RESTORE = 0xF120
WM_CLOSE = 0x0010

# 自定义"移动窗口"消息：与 gui/desktop.py WndProc 的 WM_APP_MOVERESIZE 对齐。
# 前端自绘标题栏 mousedown → 本 action；desktop.py 收到后 ReleaseCapture 并发
# WM_NCLBUTTONDOWN(HTCAPTION) 进入系统原生移动循环（拖动/Aero Snap 全原生）。
WM_APP_MOVERESIZE = 0x8001
HTCAPTION = 2


def _is_zoomed(hwnd: int) -> bool:
    return bool(ctypes.windll.user32.IsZoomed(hwnd))


def window_action(action: str, qs: dict | None = None) -> dict:
    """/api/window/<action> 路由 → 双窗口 hwnd 直控（pywebview 壳）。

    action：minimize / toggle-maximize / toggle-fullscreen / drag / close /
    is-maximized / wndproc-status / open-reader（qs['album']/qs['title']/
    qs['chapter']，仅主窗发起）。
    qs['wid']：目标窗口 id（默认 "main"；阅读窗口为 "reader"）。
    wid=main 的 close → 关闭主窗退出应用；wid=reader 的 close → 仅隐藏阅读窗。
    """
    qs = qs or {}
    wid = str(qs.get("wid") or "main")
    user32 = ctypes.windll.user32

    # ---- 独立阅读窗专属动作 ----
    if wid == "reader":
        if action == "open-reader":
            return {"ok": False, "error": "reader cannot open reader"}
        from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

        hwnd = get_reader_hwnd()
        if not hwnd:
            return {"ok": False, "error": "window not ready"}
        if action == "minimize":
            return gui_desktop.minimize_reader()
        if action == "toggle-maximize":
            return gui_desktop.toggle_reader_maximize()
        if action == "toggle-fullscreen":
            return gui_desktop.toggle_fullscreen("reader")
        if action == "wndproc-status":
            return gui_desktop.wndproc_status("reader")
        if action == "close":
            # 关闭阅读窗 = 隐藏（窗口对象常驻，再次 open-reader 复用）
            return gui_desktop.close_reader()
        if action == "is-maximized":
            return {"ok": True, "maximized": bool(user32.IsZoomed(hwnd)),
                    "fullscreen": gui_desktop.fullscreen_state("reader")}
        if action == "drag":
            try:
                if gui_desktop.start_manual_drag(hwnd):
                    return {"ok": True}
            except Exception:  # noqa: BLE001
                pass
            _post(hwnd, WM_APP_MOVERESIZE, HTCAPTION)
            return {"ok": True}
        if action == "resize":
            # 边缘缩放：WebView2 盖满客户区 → WndProc 收不到边缘 NCLBUTTONDOWN，
            # 由 DOM 边缘 mousedown（带 edges 参数）发起轮询缩放实现。
            try:
                if gui_desktop.start_manual_resize(hwnd, str(qs.get("edges") or "")):
                    return {"ok": True}
            except Exception:  # noqa: BLE001
                pass
            return {"ok": False, "error": "resize unavailable"}
        return {"ok": False, "error": f"unknown action: {action}"}

    # ---- 主窗口（ctypes 直控） ----
    if action == "open-reader":
        from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

        album = str(qs.get("album") or "").strip()
        if not album:
            return {"ok": False, "error": "missing album"}
        title = str(qs.get("title") or "").strip()
        chapter = str(qs.get("chapter") or qs.get("ch") or "").strip()
        try:
            return gui_desktop.open_reader(album, title, chapter)
        except Exception as exc:  # noqa: BLE001
            return {"ok": False, "error": str(exc)}

    hwnd = get_main_hwnd()
    if not hwnd:
        return {"ok": False, "error": "window not ready"}
    if action == "minimize":
        # 同步 ShowWindow(SW_MINIMIZE) → 系统标准最小化动画（PostMessage 偶发被吞不生效）
        user32.ShowWindow(hwnd, 6)  # SW_MINIMIZE
    elif action == "toggle-maximize":
        # 同步 ShowWindow → 走系统补间动画（PostMessage WM_SYSCOMMAND 在本壳下偶发被吞）
        user32.ShowWindow(hwnd, 9 if _is_zoomed(hwnd) else 3)  # SW_RESTORE=9 / SW_MAXIMIZE=3
    elif action == "toggle-fullscreen":
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            return gui_desktop.toggle_fullscreen("main")
        except Exception:  # noqa: BLE001
            return {"ok": False, "error": "fullscreen unavailable"}
    elif action == "wndproc-status":
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            return gui_desktop.wndproc_status("main")
        except Exception:  # noqa: BLE001
            return {"ok": False, "error": "wndproc-status unavailable"}
    elif action == "drag":
        # 主路径：WndProc 接管 WM_NCLBUTTONDOWN（标题栏 HTCAPTION 不进 DOM）。
        # 这里是兜底——WndProc 因窗口晚建等原因未安装时，前端 mousedown 仍能
        # 到达 DOM，由后端直接拉起拖动循环，保证拖动永远可用。
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            if gui_desktop.start_manual_drag(hwnd):
                return {"ok": True}
        except Exception:  # noqa: BLE001 兜底失败再退回消息路径
            pass
        _post(hwnd, WM_APP_MOVERESIZE, HTCAPTION)
    elif action == "close":
        # 走 desktop.request_close()：发 WM_CLOSE 并带 1.5s 后强制退出兜底
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            gui_desktop.request_close()
            return {"ok": True}
        except Exception:  # noqa: BLE001
            pass
        _post(hwnd, WM_CLOSE)
    elif action == "is-maximized":
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            fs = gui_desktop.fullscreen_state("main")
        except Exception:  # noqa: BLE001
            fs = False
        return {"ok": True, "maximized": bool(user32.IsZoomed(hwnd)),
                "fullscreen": fs}
    elif action == "resize":
        # 边缘缩放：WebView2 盖满客户区 → WndProc 收不到边缘 NCLBUTTONDOWN，
        # 由 DOM 边缘 mousedown（带 edges 参数）发起轮询缩放实现。
        try:
            from . import desktop as gui_desktop  # 延迟导入，避免循环依赖

            if gui_desktop.start_manual_resize(hwnd, str(qs.get("edges") or "")):
                return {"ok": True}
        except Exception:  # noqa: BLE001
            pass
        return {"ok": False, "error": "resize unavailable"}
    else:
        return {"ok": False, "error": f"unknown action: {action}"}
    return {"ok": True}


# --------------------------------------------------------------------------- #
# 打开系统文件管理器中的路径（不依赖窗口控制器，Qt 壳 / 浏览器模式均可用）
# --------------------------------------------------------------------------- #
def open_path(path: str) -> dict:
    """在系统文件管理器中打开目录/文件。

    前端「打开输出目录」按钮的桥：旧 pywebview 壳经 window.pywebview.api.open_path
    调用，Qt 壳无 pywebview 桥，统一改走本 HTTP 端点（worker 线程直接执行即可，
    不触碰 Qt 控件，无需跨线程 marshal）。
    """
    try:
        target = Path(path).expanduser()
        if not target.exists():
            return {"ok": False, "error": f"路径不存在：{target}"}
        if os.name == "nt":
            os.startfile(str(target))  # type: ignore[attr-defined]
        elif sys.platform == "darwin":
            subprocess.Popen(["open", str(target)])
        else:
            subprocess.Popen(["xdg-open", str(target)])
        return {"ok": True}
    except Exception as exc:  # noqa: BLE001 —— 打开失败不应让 worker 线程崩溃
        return {"ok": False, "error": str(exc)}


# --------------------------------------------------------------------------- #
# HTTP 路由
# --------------------------------------------------------------------------- #
ROOT = Path(__file__).resolve().parent.parent
STATIC_DIR = ROOT / "static"

_JSON_HEADERS = {"Content-Type": "application/json; charset=utf-8"}
_HTML_HEADERS = {"Content-Type": "text/html; charset=utf-8"}


def _json_response(payload: dict, status: int = 200):
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    headers = dict(_JSON_HEADERS)
    headers["Content-Length"] = str(len(body))
    return status, headers, body


class GuiHandler(SimpleHTTPRequestHandler):
    """极小路由层：/api/* JSON；其余按 static/ 静态文件回退 index.html。"""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(STATIC_DIR), **kwargs)

    def log_message(self, fmt, *args):  # 静默访问日志（GUI 应用不打控制台）
        return

    # ---- helpers ----
    def _query(self) -> dict:
        parsed = urlparse(self.path)
        return {key: values[0] for key, values in parse_qs(parsed.query).items() if values}

    def _preview_json(self, call):
        """执行预览门面调用 → JSON。PreviewError 映射为对应状态码。"""
        try:
            data = call()
        except jm_api.PreviewError as exc:
            return self._send_bytes(*_json_response(
                {"ok": False, "code": exc.code, "message": exc.message}, exc.status))
        except Exception as exc:  # noqa: BLE001 —— 未知异常不可让 worker 线程崩溃
            import traceback
            traceback.print_exc()
            return self._send_bytes(*_json_response(
                {"ok": False, "code": "internal", "message": f"内部错误：{type(exc).__name__}"}, 500))
        return self._send_bytes(*_json_response({"ok": True, **data}))

    def _preview_bytes(self, call):
        """执行预览门面调用 → 原始图片字节。错误返回 JSON（含状态码）。"""
        try:
            content, mime = call()
        except jm_api.PreviewError as exc:
            return self._send_bytes(*_json_response(
                {"ok": False, "code": exc.code, "message": exc.message}, exc.status))
        except Exception as exc:  # noqa: BLE001
            import traceback
            traceback.print_exc()
            return self._send_bytes(*_json_response(
                {"ok": False, "code": "internal", "message": f"内部错误：{type(exc).__name__}"}, 500))

        if not content:
            return self._send_bytes(*_json_response(
                {"ok": False, "code": "network", "message": "上游返回空图片数据"}, 502))
        headers = {
            "Content-Type": mime or "application/octet-stream",
            "Content-Length": str(len(content)),
            "Cache-Control": "private, max-age=3600",
        }
        return self._send_bytes(200, headers, content)

    def _read_json_body(self) -> dict:
        try:
            length = int(self.headers.get("Content-Length") or 0)
            if length <= 0:
                return {}
            raw = self.rfile.read(length)
            return json.loads(raw.decode("utf-8"))
        except (ValueError, json.JSONDecodeError):
            return {}

    def _send_bytes(self, status: int, headers: dict, body: bytes) -> None:
        self.send_response(status)
        for key, value in headers.items():
            self.send_header(key, value)
        self.end_headers()
        if body:
            self.wfile.write(body)

    # ---- route handlers ----
    def do_GET(self):  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path

        if path == "/api/health":
            return self._send_bytes(*_json_response({
                "ok": True,
                "app": "JMComic-Downloader-GUI",
                "outputDir": str(default_download_directory()),
            }))

        if path == "/api/settings":
            return self._send_bytes(*_json_response({"settings": current_settings()}))

        if path == "/api/open-path":
            return self._send_bytes(*_json_response(open_path(str(self._query().get("path") or ""))))

        window_match = path.startswith("/api/window/")
        if window_match:
            action = path.rsplit("/", 1)[-1]
            return self._send_bytes(*_json_response(window_action(action, self._query())))

        # ---- 在线预览（只读，不落盘） ----
        if path == "/api/search":
            qs = self._query()
            try:
                page = int(qs.get("page") or 1)
            except ValueError:
                page = 1
            return self._preview_json(lambda: PREVIEW.search(qs.get("q") or "", page))

        if path == "/api/album":
            return self._preview_json(lambda: PREVIEW.album(self._query().get("id") or ""))

        if path == "/api/photo":
            return self._preview_json(lambda: PREVIEW.photo(self._query().get("id") or ""))

        if path == "/api/preview/status":
            return self._send_bytes(*_json_response(PREVIEW.status()))

        if path == "/api/cover":
            qs = self._query()
            album_id = qs.get("id") or qs.get("aid") or ""
            return self._preview_bytes(lambda: PREVIEW.cover(album_id, qs.get("size") or ""))

        if path == "/api/image":
            qs = self._query()
            try:
                index = int(qs.get("index") or 1)
            except ValueError:
                index = 0
            return self._preview_bytes(lambda: PREVIEW.image(qs.get("photo") or "", index))

        # ---- 下载任务（worker 子进程） ----
        if path == "/api/history":
            return self._send_bytes(*_json_response({"ok": True, "history": JOBS.history()}))
        if path == "/api/jobs":
            return self._send_bytes(*_json_response(
                {"ok": True, "jobs": JOBS.list(), "stats": JOBS.stats()}))
        if path.startswith("/api/jobs/"):
            rest = path[len("/api/jobs/"):]
            job_id, _, tail = rest.partition("/")
            rec = JOBS.get(job_id) if job_id else None
            if rec is None:
                return self._send_bytes(*_json_response({"ok": False, "code": "not_found",
                                                         "message": f"任务不存在：{job_id}"}, 404))
            if tail in ("", "/"):
                return self._send_bytes(*_json_response({"ok": True, "job": rec.to_summary()}))
            if tail == "events":
                return self._stream_job_events(rec)
            if tail == "log":
                try:
                    body = Path(rec.log_path).read_bytes()
                except OSError:
                    body = b""
                return self._send_bytes(200, {
                    "Content-Type": "text/plain; charset=utf-8",
                    "Content-Length": str(len(body)),
                }, body)
            return self._send_bytes(*_json_response({"ok": False, "error": f"unknown subpath: {tail}"}, 404))

        # 静态资源；不存在回退 index.html（SPA 路由）
        return self._serve_static(path)

    def do_POST(self):  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path

        if path == "/api/settings":
            body = self._read_json_body()
            settings = body.get("settings") or {}
            merged = dict(_SETTINGS_CACHE)
            merged.update(settings)
            save_settings(merged)
            _refresh_settings_cache()
            return self._send_bytes(*_json_response({"ok": True, "settings": current_settings()}))

        if path == "/api/jobs":
            body = self._read_json_body()
            kind = (body.get("kind") or "").strip()
            ids = body.get("ids") or []
            options = body.get("options") or {}
            if not isinstance(ids, list) or not ids:
                return self._send_bytes(*_json_response(
                    {"ok": False, "code": "bad_request", "message": "ids 必须为非空数组"}, 400))
            for v in ids:
                if not isinstance(v, str) or not v.strip():
                    return self._send_bytes(*_json_response(
                        {"ok": False, "code": "bad_request", "message": "ids 必须全部为非空字符串"}, 400))
            output_dir = (options.get("outputDir") or "").strip() or str(default_download_directory())
            try:
                job_opts = gui_jobs.JobOptions(
                    kind=kind,
                    ids=[str(v).strip() for v in ids],
                    output_dir=output_dir,
                    client=options.get("client") or "api",
                    proxy=options.get("proxy") or "",
                    image_format=options.get("imageFormat") or "original",
                    image_threads=int(options.get("imageThreads") or 20),
                    photo_threads=int(options.get("photoThreads") or 4),
                    option_file=options.get("optionFile") or "",
                )
            except ValueError as exc:
                return self._send_bytes(*_json_response(
                    {"ok": False, "code": "bad_request", "message": str(exc)}, 400))
            if kind not in ("album", "photo"):
                return self._send_bytes(*_json_response(
                    {"ok": False, "code": "bad_request", "message": "kind 必须是 album 或 photo"}, 400))
            rec = JOBS.submit(job_opts)
            return self._send_bytes(*_json_response({"ok": True, "job": rec.to_summary()}, 202))

        return self._send_bytes(*_json_response({"ok": False, "error": f"POST {path} not found"}, 404))

    def do_DELETE(self):  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/api/history":
            n = JOBS.clear_history()
            return self._send_bytes(*_json_response({"ok": True, "cleared": n}))
        if path.startswith("/api/history/"):
            job_id = path[len("/api/history/"):]
            if not job_id:
                return self._send_bytes(*_json_response({"ok": False, "error": "bad path"}, 404))
            ok = JOBS.remove_history(job_id)
            if not ok:
                return self._send_bytes(*_json_response(
                    {"ok": False, "code": "not_found", "message": f"历史记录不存在：{job_id}"}, 404))
            return self._send_bytes(*_json_response({"ok": True, "removed": True}))
        if path.startswith("/api/jobs/"):
            rest = path[len("/api/jobs/"):]
            job_id, _, tail = rest.partition("/")
            if tail in ("", "/"):
                rec = JOBS.get(job_id)
                if rec is None:
                    return self._send_bytes(*_json_response(
                        {"ok": False, "code": "not_found", "message": f"任务不存在：{job_id}"}, 404))
                if rec.state in gui_jobs.TERMINAL_STATES:
                    # 终态任务：从活动列表清除（历史记录保留在 /api/history）
                    JOBS.remove(job_id)
                    return self._send_bytes(*_json_response({"ok": True, "removed": True}))
                ok = JOBS.cancel(job_id)
                if not ok:
                    return self._send_bytes(*_json_response(
                        {"ok": False, "code": "not_found", "message": f"任务不存在：{job_id}"}, 404))
                return self._send_bytes(*_json_response({"ok": True, "cancelled": True}))
        return self._send_bytes(*_json_response({"ok": False, "error": f"DELETE {path} not found"}, 404))

    def _stream_job_events(self, rec) -> None:
        """以 Server-Sent Events 输出任务的实时日志/状态事件，直至终态。"""
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-cache, no-store")
            self.send_header("X-Accel-Buffering", "no")
            self.send_header("Connection", "close")
            self.end_headers()
        except Exception:
            return

        q: queue.Queue = queue.Queue()
        rec.subscribers.append(q)
        sent_index = 0
        # 立即重放已有事件，便于前端连接时拿到完整历史
        initial = list(rec.events)
        try:
            for ev in initial:
                sent_index += 1
                self._write_sse(ev)
        except Exception:
            rec.subscribers.remove(q)
            return

        last_keepalive = time.time()
        try:
            while rec.state not in gui_jobs.TERMINAL_STATES:
                try:
                    ev = q.get(timeout=0.5)
                    self._write_sse(ev)
                    sent_index += 1
                except queue.Empty:
                    pass
                if time.time() - last_keepalive > 15.0:
                    try:
                        self.wfile.write(b": keep-alive\n\n")
                        self.wfile.flush()
                    except Exception:
                        break
                    last_keepalive = time.time()
            # 终态：把剩余事件刷出再关闭
            for ev in list(rec.events)[sent_index:]:
                try:
                    self._write_sse(ev)
                except Exception:
                    break
        finally:
            try:
                rec.subscribers.remove(q)
            except ValueError:
                pass

    def _write_sse(self, ev: dict) -> None:
        ev_name = ev.get("type") or "message"
        payload = json.dumps(ev, ensure_ascii=False)
        chunk = f"event: {ev_name}\ndata: {payload}\n\n".encode("utf-8")
        self.wfile.write(chunk)
        try:
            self.wfile.flush()
        except Exception:
            raise

    def _serve_static(self, path: str) -> None:
        rel = path.lstrip("/")
        if not rel or rel.endswith("/"):
            rel = "index.html"
        candidate = (STATIC_DIR / rel).resolve()
        try:
            candidate.relative_to(STATIC_DIR.resolve())
        except ValueError:
            return self._send_bytes(*_json_response({"ok": False, "error": "bad path"}, 404))

        if candidate.is_file():
            content_type = {
                ".html": "text/html; charset=utf-8",
                ".css": "text/css; charset=utf-8",
                ".js": "application/javascript; charset=utf-8",
                ".svg": "image/svg+xml",
                ".ico": "image/x-icon",
                ".png": "image/png",
                ".woff2": "font/woff2",
            }.get(candidate.suffix.lower(), "application/octet-stream")
            try:
                body = candidate.read_bytes()
            except OSError:
                return self._send_bytes(*_json_response({"ok": False, "error": "read failed"}, 500))
            headers = {"Content-Type": content_type, "Content-Length": str(len(body))}
            if candidate.suffix in (".html", ".css", ".js"):
                headers["Cache-Control"] = "no-store"
            return self._send_bytes(200, headers, body)

        # SPA 回退
        fallback = STATIC_DIR / "index.html"
        try:
            body = fallback.read_bytes()
        except OSError:
            return self._send_bytes(404, {}, b"not found")
        return self._send_bytes(200, dict(_HTML_HEADERS, **{"Content-Length": str(len(body))}), body)


class GuiServer:
    """线程化本地服务：http://127.0.0.1:<port>"""

    def __init__(self, port: int = 0):
        self.httpd = ThreadingHTTPServer(("127.0.0.1", port), GuiHandler)
        self.httpd.daemon_threads = True  # 网络阻塞时退出不悬挂
        self.port: int = self.httpd.server_address[1]
        self.thread: threading.Thread | None = None

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def start(self) -> None:
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()

    def shutdown(self) -> None:
        try:
            self.httpd.shutdown()
            self.httpd.server_close()
        except OSError:
            pass


# 在线预览门面：设置随 current_settings() 热更新（改代理/客户端后下一请求生效）
PREVIEW = jm_api.PreviewService(current_settings)


# 下载任务管理：单实例，限制并发避免资源/带宽打满；进程内事件流由 manager 持有。
# 历史记录默认落在系统数据目录（JM_HISTORY_PATH 可覆盖，供离线 E2E/测试隔离）。
# root 必须是真实应用目录（frozen = exe 同目录）：worker 子进程以它为 cwd——
# 若 frozen 下误用 _MEIPASS，worker 存续期间会持住临时解包目录的句柄，
# 主程序退出时 PyInstaller 删不掉 _MEIxxx → 弹 "Failed to remove temporary directory"。
JOBS = gui_jobs.JobManager(
    root=app_directory(),
    max_concurrent=2,
    history_path=os.environ.get("JM_HISTORY_PATH") or str(_app_data_dir() / "jobs_history.json"),
)


def create_server(port: int = 0) -> GuiServer:
    return GuiServer(port)
