# -*- coding: utf-8 -*-
"""JMComic Downloader GUI · 桌面壳（pywebview 6 + WebView2，双窗口）

窗口模型对齐 Novelist 的打磨成果（从 PySide6 回退）：
- frameless（无 WS_CAPTION）+ easy_drag=False，完全自绘标题栏；保留 WS_THICKFRAME
  仅供边缘 resize 手柄视觉、min/max/sysmenu 供自绘按钮 / Alt+Space 菜单；
- WndProc 子类化：拖动 / Aero Snap / 双击最大化 / 边缘 resize / Alt+Space 系统
  菜单全部由 WndProc + 自绘逻辑接管；
- WM_NCCALCSIZE：最大化时把 rgrc[0] 修正为 rcWork（不盖任务栏），frameless 下
  仍正确（最大化态需避开任务栏）；
- WM_NCHITTEST：标题栏区域 HTCAPTION，右上按钮区 HTCLIENT（可点击），
  边缘/四角 resize 手柄；WM_GETMINMAXINFO 设最小尺寸；
- ⚠️ HTCAPTION 命中后鼠标消息由顶层窗口接管（不进 WebView2，DOM 拿不到事件），
  故 WM_NCLBUTTONDOWN / WM_NCLBUTTONDBLCLK 必须在 WndProc 内自行处理：
  → 拖动走自绘轮询循环（_manual_move_drag），双击走 WM_SYSCOMMAND 最大化/还原；
- 双窗口（main + reader，均在 webview.start() 前以 hidden=True 预创建——
  pywebview 6 不允许运行期新增窗口）：
  · main   主窗口：hidden 启动，loaded/5s 兜底后显示并居中；关闭 → 退出应用；
  · reader 独立阅读窗：全程常驻隐藏，open-reader 动作 show + 导航；WndProc
    拦截其 WM_CLOSE → 转 SW_HIDE（"关闭"即隐藏，窗口对象不销毁，再次打开复用）；
- WebView2 数据隔离：storage_path 指向应用专属缓存目录（exe 同目录
  .webview2-cache，源码模式 = 仓库根），绝不用 pywebview 默认共享的
  %APPDATA%\\pywebview —— 不污染/不被污染系统上其它 WebView2 实例；
- 看门狗线程：主窗口曾出现后消失 → 立即 os._exit（防 WebView2 清理挂起导致进程残留）。
"""
from __future__ import annotations

import ctypes
import os
import sys
import threading
import time as _time
import traceback as _traceback
from pathlib import Path

import webview

from . import server as gui_server

APP_TITLE = "JMComic Downloader GUI"
READER_TITLE = "JMComic Reader"
WIDTH, HEIGHT = 1280, 820
MIN_W, MIN_H = 1020, 640
READER_W, READER_H = 1220, 860
READER_MIN_W, READER_MIN_H = 900, 600
TITLEBAR_H = 38           # HTML 自绘标题栏高度(px)，须与 static/css 一致
BTN_AREA_W = 140          # 右侧窗口按钮区宽度(px)，该区域不参与拖动
EDGE = 8                  # 窗口边缘可拖拽调大小宽度(px)


class Api:
    """pywebview js_api 桥（保留扩展位；主要交互走 HTTP API）"""

    def __init__(self, srv):
        self.srv = srv

    def app_info(self):
        return {"title": APP_TITLE, "port": self.srv.port}

    def open_path(self, path: str):
        """打开系统资源管理器/文件（如“打开 PDF 目录”）。"""
        try:
            target = Path(path)
            if target.is_file():
                os.startfile(str(target))  # type: ignore[attr-defined]
            else:
                target.mkdir(parents=True, exist_ok=True)
                os.startfile(str(target))  # type: ignore[attr-defined]
            return {"ok": True}
        except OSError as exc:
            return {"ok": False, "error": str(exc)}


# --------------------------------------------------------------------------- #
# 窗口逻辑（WndProc 子类化，按窗口独立安装）
# --------------------------------------------------------------------------- #
class _POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


class _RECT(ctypes.Structure):
    _fields_ = [("left", ctypes.c_long), ("top", ctypes.c_long),
                ("right", ctypes.c_long), ("bottom", ctypes.c_long)]


VK_LBUTTON = 0x01
SWP_NOSIZE, SWP_NOZORDER, SWP_NOACTIVATE = 0x0001, 0x0004, 0x0010
WM_SYSCOMMAND = 0x0112
SC_MAXIMIZE, SC_RESTORE = 0xF030, 0xF120
SC_MOVE, SC_SIZE = 0xF010, 0xF000   # SC_MOVE|HTCAPTION(0xF012)=系统移动循环；SC_SIZE|HTxxx=系统缩放循环
_drag_active: list = [False]

# 原生窗口行为开关与状态（按窗口标题索引）
# ⚠️ 实测（2026-09-04）：WinForms 宿主吞掉 SC_MOVE/SC_SIZE 系统循环，且
# WebView2 子窗口盖满客户区使 WM_NCLBUTTONDOWN 到不了 WndProc → 原生拖动/
# 缩放路径不可用，统一走 start_manual_drag/start_manual_resize 轮询实现。
_NATIVE_DRAG = False           # True 时 WM_NCLBUTTONDOWN(HTCAPTION) 尝试系统循环（死路径，保留备用）
_fullscreen: dict = {}         # title -> 是否全屏（自建全屏，不走 pywebview toggle_fullscreen）
_restore_box: dict = {}        # title -> (l, t, r, b, was_maximized) 退出全屏时还原
_caption_stripped: set = set() # 已降级摘掉 WS_CAPTION 的窗口（WndProc 装不上的保底）


def _manual_move_drag(h) -> None:
    """自绘拖动循环（后台线程轮询光标 + SetWindowPos 跟随，Aero Snap 语义自绘）。

    ⚠️ 实测：pywebview 的 WinForms 宿主吞掉 SC_MOVE/SC_SIZE（WM_NCLBUTTONDOWN
    也因 WebView2 子窗口盖满客户区而不会到达本 WndProc）→ 系统移动/缩放循环
    不可用，只能轮询模拟。Snap 语义（对齐原生习惯）：
      - 顶部停留 ≥ _SNAP_MAX_HOLD → SC_MAXIMIZE 自动最大化（带原生补间动画）；
      - 左/右边缘释放 → 吸附到该显示器工作区左/右半屏（Aero Snap 效果）。
    """
    user32 = ctypes.windll.user32
    if _drag_active[0]:
        return
    _drag_active[0] = True
    try:
        pt = _POINT()
        if not user32.GetCursorPos(ctypes.byref(pt)):
            return
        rr = _RECT()
        if not user32.GetWindowRect(h, ctypes.byref(rr)):
            return
        grab_x = pt.x - rr.left           # 抓取点相对窗口左上角的偏移（保持跟手）
        grab_y = pt.y - rr.top
        w, hgt = rr.right - rr.left, rr.bottom - rr.top
        top_since: float | None = None    # 进入顶部吸附区的时刻
        while user32.GetAsyncKeyState(VK_LBUTTON) & 0x8000:
            _time.sleep(0.016)
            if not user32.GetCursorPos(ctypes.byref(pt)):
                break
            nx, ny = pt.x - grab_x, pt.y - grab_y
            mon = user32.MonitorFromPoint(ctypes.byref(pt), 2)
            mi = MONITORINFO()
            mi.cbSize = ctypes.sizeof(MONITORINFO)
            if not user32.GetMonitorInfoW(mon, ctypes.byref(mi)):
                break
            wa = mi.rcWork
            # 顶部停留 → 自动最大化（预留给系统补间动画）
            if (not user32.IsZoomed(h) and ny <= wa.top + 2
                    and mi.rcMonitor.top <= pt.y <= wa.top + 30):
                if top_since is None:
                    top_since = _time.time()
                elif _time.time() - top_since >= _SNAP_MAX_HOLD:
                    # ShowWindow(SW_MAXIMIZE) 跨线程安全、同步生效（实测 PostMessage
                    # WM_SYSCOMMAND 在本壳下偶发被 GUI 队列吞掉不生效）
                    _log(f"[drag] 顶部停留触发最大化 hold={_time.time() - top_since:.2f}s")
                    try:
                        user32.ShowWindow(h, 3)  # SW_MAXIMIZE
                    except Exception:  # noqa: BLE001
                        pass
                    return
            else:
                top_since = None
            user32.SetWindowPos(h, 0, nx, ny, 0, 0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
        # —— 松开左键：边缘吸附判定（左/右半屏） ——
        if not user32.GetCursorPos(ctypes.byref(pt)):
            return
        nx, ny = pt.x - grab_x, pt.y - grab_y
        mon = user32.MonitorFromPoint(ctypes.byref(pt), 2)
        mi = MONITORINFO()
        mi.cbSize = ctypes.sizeof(MONITORINFO)
        if user32.GetMonitorInfoW(mon, ctypes.byref(mi)) and not user32.IsZoomed(h):
            wa = mi.rcWork
            # 半屏宽 = max(工作区一半, 窗口最小宽)—— 带最小尺寸的窗口贴边时
            # 原生同样按最小宽吸附（本应用 UI 最小宽 1020px，1920 屏半屏 960
            # 会被钳到 1020，属预期；大屏则得标准半屏）。
            mm = MINMAXINFO()
            mm_lparam = ctypes.cast(ctypes.byref(mm), ctypes.c_void_p).value
            user32.SendMessageW(h, WM_GETMINMAXINFO, 0, mm_lparam)
            min_w = max(200, int(mm.ptMinTrackSize.x))
            half = max((wa.right - wa.left) // 2, min_w)
            hh = wa.bottom - wa.top
            if nx <= wa.left + 6:                     # 贴左 → 左半屏
                user32.SetWindowPos(h, 0, wa.left, wa.top, half, hh,
                                    SWP_NOZORDER | SWP_NOACTIVATE)
                _log(f"[drag] 吸附左半屏 → ({wa.left},{wa.top},{half}x{hh})")
            elif nx + w >= wa.right - 6:              # 贴右 → 右半屏
                user32.SetWindowPos(h, 0, wa.right - half, wa.top, half, hh,
                                    SWP_NOZORDER | SWP_NOACTIVATE)
                _log(f"[drag] 吸附右半屏 → ({wa.right - half},{wa.top},{half}x{hh})")
    except Exception:  # noqa: BLE001 拖动循环尽力而为
        pass
    finally:
        _drag_active[0] = False


_SNAP_MAX_HOLD = 0.35     # 顶部停留多久触发最大化（秒）
_resize_active: list = [False]


def start_manual_resize(hwnd, edges: str) -> bool:
    """边缘缩放（轮询实现，替代被 WinForms 吞掉的 SC_SIZE 系统循环）。

    :param edges: 前端传来的拖拽边，如 "right" / "bottom" / "top,left"。
    """
    try:
        h = hwnd if isinstance(hwnd, ctypes.c_void_p) else ctypes.c_void_p(int(hwnd))
        user32 = ctypes.windll.user32
        if user32.IsZoomed(h) or user32.IsIconic(h):
            return False
        edges = (edges or "").strip().lower()
        if not edges or edges == "none":
            return False
        threading.Thread(target=_manual_resize_loop,
                         args=(h, edges.split(",")), daemon=True).start()
        return True
    except Exception:  # noqa: BLE001
        return False


def _manual_resize_loop(h, edges: list[str]) -> None:
    """轮询缩放：按光标位移调整被拖边，直到松开左键。

    WM_NCLBUTTONDOWN 不会到达 WndProc（WebView2 盖满客户区），且 SC_SIZE 被
    WinForms 吞掉 → 由 DOM 边缘 mousedown → /api/window/resize?edges=… 发起，
    本循环以 SetWindowPos 实时改尺寸，并遵守 WM_GETMINMAXINFO 的最小尺寸。
    """
    user32 = ctypes.windll.user32
    if _resize_active[0]:
        return
    _resize_active[0] = True
    try:
        pt = _POINT()
        if not user32.GetCursorPos(ctypes.byref(pt)):
            _log(f"[resize] 退出：GetCursorPos 失败")
            return
        s_l, s_t = pt.x, pt.y
        rr = _RECT()
        if not user32.GetWindowRect(h, ctypes.byref(rr)):
            _log(f"[resize] 退出：GetWindowRect 失败")
            return
        base = (rr.left, rr.top, rr.right, rr.bottom)
        # 最小尺寸：问窗口自身（WM_GETMINMAXINFO 已由 WndProc 按需填 min track）
        mm = MINMAXINFO()
        mm_lparam = ctypes.cast(ctypes.byref(mm), ctypes.c_void_p).value  # LPARAM(int64)
        user32.SendMessageW(h, WM_GETMINMAXINFO, 0, mm_lparam)
        min_w = max(200, int(mm.ptMinTrackSize.x))
        min_h = max(160, int(mm.ptMinTrackSize.y))
        has_l = "left" in edges
        has_r = "right" in edges
        has_t = "top" in edges
        has_b = "bottom" in edges
        _log(f"[resize] 启动 edges={edges} base={base} min={min_w}x{min_h} "
             f"cursor=({s_l},{s_t}) lmb={bool(user32.GetAsyncKeyState(VK_LBUTTON) & 0x8000)}")
        if not (has_l or has_r or has_t or has_b):
            _log(f"[resize] 退出：无可拖边 {edges}")
            return
        n = 0
        while user32.GetAsyncKeyState(VK_LBUTTON) & 0x8000:
            _time.sleep(0.016)
            if not user32.GetCursorPos(ctypes.byref(pt)):
                break
            dx, dy = pt.x - s_l, pt.y - s_t
            l, t, r, b = base
            if has_l:
                l = min(base[2] - min_w, base[0] + dx)
            if has_r:
                r = max(base[0] + min_w, base[2] + dx)
            if has_t:
                t = min(base[3] - min_h, base[1] + dy)
            if has_b:
                b = max(base[1] + min_h, base[3] + dy)
            user32.SetWindowPos(h, 0, l, t, r - l, b - t,
                                SWP_NOZORDER | SWP_NOACTIVATE)
            n += 1
        _log(f"[resize] 结束 iter={n}")
    except Exception:  # noqa: BLE001 缩放循环尽力而为
        _log(f"[resize] 异常：{_traceback.format_exc()}")
    finally:
        _resize_active[0] = False


def start_manual_drag(hwnd) -> bool:
    """外部（server 的 /api/window/drag）发起拖动的兜底入口。

    WndProc 正常时 WM_NCLBUTTONDOWN 已原生处理，前端根本收不到标题栏鼠标
    事件，此入口不会走到；仅当 WndProc 因故未安装（窗口晚建等）时兜底，
    保证拖动在任意情况下都可用。
    """
    try:
        h = hwnd if isinstance(hwnd, ctypes.c_void_p) else ctypes.c_void_p(int(hwnd))
        user32 = ctypes.windll.user32
        if user32.IsZoomed(h) or user32.IsIconic(h):
            return False
        threading.Thread(target=_manual_move_drag, args=(h,), daemon=True).start()
        return True
    except Exception:  # noqa: BLE001
        return False


# 每窗口的 WndProc 状态：orig proc / 异常计数 / 是否已死（键 = 窗口标题）
_orig_wndproc: dict = {}
_state_box: dict = {}
_wndproc_refs: dict = {}
_wnd_installed: dict = {}


class MINMAXINFO(ctypes.Structure):
    _fields_ = [("ptReserved", _POINT), ("ptMaxSize", _POINT),
                ("ptMaxPosition", _POINT), ("ptMinTrackSize", _POINT),
                ("ptMaxTrackSize", _POINT)]


class NCCALCSIZE_PARAMS(ctypes.Structure):
    _fields_ = [("rgrc", _RECT * 3), ("lppos", ctypes.c_void_p)]


class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", ctypes.c_ulong), ("rcMonitor", _RECT),
                ("rcWork", _RECT), ("dwFlags", ctypes.c_ulong)]


WM_NCCALCSIZE, WM_NCHITTEST = 0x0083, 0x0084
WM_GETMINMAXINFO, WM_NCLBUTTONDOWN = 0x0024, 0x00A1
WM_NCLBUTTONDBLCLK = 0x00A3
WM_APP_MOVERESIZE = 0x8001  # server /api/window/drag 兜底 → 此消息
WM_CLOSE = 0x0010
SW_HIDE = 0
HTCLIENT, HTCAPTION = 1, 2
HTLEFT, HTRIGHT, HTTOP = 10, 11, 12
HTTOPLEFT, HTTOPRIGHT, HTBOTTOM = 13, 14, 15
HTBOTTOMLEFT, HTBOTTOMRIGHT = 16, 17


def _setup_user32() -> None:
    """一次性声明 ctypes 签名（进程内只做一次）。"""
    if getattr(_setup_user32, "_done", False):
        return
    _setup_user32._done = True  # type: ignore[attr-defined]
    user32 = ctypes.windll.user32
    LRESULT = ctypes.c_longlong
    user32.FindWindowW.restype = ctypes.c_void_p
    user32.SetWindowLongPtrW.restype = LRESULT
    user32.GetWindowLongPtrW.restype = LRESULT
    user32.CallWindowProcW.restype = LRESULT
    user32.SendMessageW.restype = LRESULT
    user32.SetWindowLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p]
    user32.GetWindowLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int]
    user32.CallWindowProcW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint,
                                       ctypes.c_uint64, ctypes.c_int64]
    user32.GetCursorPos.restype = ctypes.c_int
    user32.GetCursorPos.argtypes = [ctypes.POINTER(_POINT)]
    user32.GetAsyncKeyState.restype = ctypes.c_short
    user32.GetAsyncKeyState.argtypes = [ctypes.c_int]
    user32.GetWindowRect.restype = ctypes.c_int
    user32.GetWindowRect.argtypes = [ctypes.c_void_p, ctypes.POINTER(_RECT)]
    user32.SetWindowPos.restype = ctypes.c_int
    user32.SetWindowPos.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int, ctypes.c_int,
                                    ctypes.c_int, ctypes.c_int, ctypes.c_uint]
    user32.SendMessageW.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint64, ctypes.c_int64]


def _apply_dwm(hwnd, *, rounded: bool = True) -> None:
    """DWM：关闭非客户区渲染，根治 frameless 下的 1px accent/白细边残留。

    frameless 壳即使摘掉 WS_CAPTION/WS_BORDER，为保留边缘缩放手柄仍带
    WS_THICKFRAME，此时 Win11 的 DWM 会在窗口四周绘制非客户区（accent 色
    细边框 + 圆角）——即 UI 顶部那条 1px 蓝/白线。把它们画出来的是 DWM，不是
    HTML。置 DWMWA_NCRENDERING_POLICY=DWMNCRP_DISABLED 后，DWM 不再绘制任何
    非客户区；rounded 再通过 DWMWA_WINDOW_CORNER_PREFERENCE 显式声明圆角，
    让悬浮卡片风格与自绘栏一致（全屏态传 rounded=False 还原直角）。
    """
    try:
        dwm = ctypes.windll.dwmapi

        def _set(attr: int, val: int) -> None:
            v = ctypes.c_int(val)
            dwm.DwmSetWindowAttribute(ctypes.c_void_p(int(hwnd)), attr,
                                      ctypes.byref(v), ctypes.sizeof(v))

        _set(2, 1)   # DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED
        _set(33, 2 if rounded else 0)  # DWMWA_WINDOW_CORNER_PREFERENCE: ROUND(2)/DEFAULT(0)
    except Exception:  # noqa: BLE001 dwmapi 缺失或旧版系统不支持时静默降级
        pass


def _make_wndproc(title: str, min_w: int, min_h: int, intercept_close: bool):
    """按窗口创建 WndProc 回调。

    :param intercept_close: True 时 WM_CLOSE → SW_HIDE（reader 窗，常驻复用）；
                            False 时 WM_CLOSE 交还系统（main 窗 → 正常关闭退出）。
    """
    _setup_user32()
    user32 = ctypes.windll.user32
    LRESULT = ctypes.c_longlong
    WNDPROC_T = ctypes.WINFUNCTYPE(LRESULT, ctypes.c_void_p, ctypes.c_uint,
                                   ctypes.c_uint64, ctypes.c_int64)
    state = {"rect": None, "rect_ts": 0.0, "err": 0, "dead": False}
    _state_box[title] = state

    @WNDPROC_T
    def wndproc(h, msg, wparam, lparam):
        try:
            if state["dead"]:
                return user32.CallWindowProcW(_orig_wndproc[title], h, msg, wparam, lparam)
            if msg == WM_NCCALCSIZE:
                if _fullscreen.get(title):
                    return 0   # 自建全屏：窗口 rect 已是显示器全尺寸，跳过 rcWork 修正（否则盖不住任务栏）
                if wparam and user32.IsZoomed(h):
                    params = ctypes.cast(lparam, ctypes.POINTER(NCCALCSIZE_PARAMS)).contents
                    mon = user32.MonitorFromWindow(h, 2)
                    mi = MONITORINFO()
                    mi.cbSize = ctypes.sizeof(MONITORINFO)
                    if user32.GetMonitorInfoW(mon, ctypes.byref(mi)):
                        params.rgrc[0] = mi.rcWork
                return 0
            if msg == WM_NCLBUTTONDOWN:
                # HTCAPTION 命中后鼠标消息交给顶层窗口（不进 WebView2，DOM 拿不到
                # mousedown）。主路径：ReleaseCapture + SC_MOVE|HTCAPTION 启动系统
                # 原生移动循环（跟手 + Aero Snap：拖到边缘半屏吸附、拖到顶部最大化
                # 预览）。⚠️ 必须 SendMessage 同步进入（modal loop 嵌在本次派发内，
                # 循环结束即返回）；PostMessage 异步会被 WinForms 消息泵延迟/吞掉。
                # 回退：自绘轮询拖动（start_manual_drag）。
                if wparam == HTCAPTION:
                    if _NATIVE_DRAG:
                        user32.ReleaseCapture()
                        user32.SendMessageW(h, WM_SYSCOMMAND, SC_MOVE + HTCAPTION, lparam)
                        return 0
                    start_manual_drag(h)
                    return 0
                # 边缘缩放兜底：统一走系统缩放循环（SC_SIZE|HTxxx），
                # 不依赖 WinForms 是否把消息转发给 DefWindowProc。
                if wparam in (HTLEFT, HTRIGHT, HTTOP, HTBOTTOM,
                              HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT):
                    user32.ReleaseCapture()
                    user32.SendMessageW(h, WM_SYSCOMMAND, SC_SIZE + wparam, lparam)
                    return 0
            if msg == WM_NCLBUTTONDBLCLK:
                # 标题栏双击最大化/还原（同理：DOM dblclick 收不到）
                if wparam == HTCAPTION:
                    user32.SendMessageW(h, WM_SYSCOMMAND,
                                        SC_RESTORE if user32.IsZoomed(h) else SC_MAXIMIZE, 0)
                    return 0
            if msg == WM_APP_MOVERESIZE:
                start_manual_drag(h)
                return 0
            if msg == WM_NCHITTEST:
                if _fullscreen.get(title):
                    return HTCLIENT   # 全屏态：不提供边缘缩放手柄
                x = ctypes.c_short(lparam & 0xFFFF).value
                y = ctypes.c_short((lparam >> 16) & 0xFFFF).value
                now = _time.time()
                if state["rect"] is None or now - state["rect_ts"] > 0.05:
                    rr = _RECT()
                    user32.GetWindowRect(h, ctypes.byref(rr))
                    state["rect"] = (rr.left, rr.top, rr.right - rr.left, rr.bottom - rr.top)
                    state["rect_ts"] = now
                lft, top, w, hgt = state["rect"]
                cx, cy = x - lft, y - top
                if not user32.IsZoomed(h):
                    if cx <= EDGE and cy <= EDGE:
                        return HTTOPLEFT
                    if cx >= w - EDGE and cy <= EDGE:
                        return HTTOPRIGHT
                    if cx <= EDGE and cy >= hgt - EDGE:
                        return HTBOTTOMLEFT
                    if cx >= w - EDGE and cy >= hgt - EDGE:
                        return HTBOTTOMRIGHT
                    if cy <= EDGE:
                        return HTTOP
                    if cy >= hgt - EDGE:
                        return HTBOTTOM
                    if cx <= EDGE:
                        return HTLEFT
                    if cx >= w - EDGE:
                        return HTRIGHT
                if cy < TITLEBAR_H and cx < w - BTN_AREA_W:
                    return HTCAPTION
                return HTCLIENT
            if msg == WM_GETMINMAXINFO:
                mm = ctypes.cast(lparam, ctypes.POINTER(MINMAXINFO)).contents
                mm.ptMinTrackSize.x = min_w
                mm.ptMinTrackSize.y = min_h
                return 0
            if msg == WM_CLOSE and intercept_close:
                # reader 窗“关闭”= 隐藏（pywebview 窗口须在 start() 前创建，
                # 销毁后无法重建；隐藏后再次 open-reader 直接复用）。
                user32.ShowWindow(h, SW_HIDE)
                return 0
            if msg in (0x0003, 0x0005, 0x0007):  # WM_MOVE/WM_SIZE/WM_SHOWWINDOW
                state["rect"] = None
        except Exception:  # noqa: BLE001 快速失败：回调异常过多则恢复原 proc
            state["err"] += 1
            if state["err"] >= 20 and _orig_wndproc.get(title):
                state["dead"] = True
                try:
                    user32.SetWindowLongPtrW(h, -4, _orig_wndproc[title])
                except Exception:  # noqa: BLE001
                    pass
                # 还原原 proc 后清掉“已安装”标记：_install_window_logic 的
                # 轮询线程会带着全新 WndProc 重装，避免死透后系统框回归。
                _wnd_installed.pop(title, None)
                _log(f"[wndproc] {title} 回调异常过频已还原原 proc，启动自愈重装")
                try:
                    threading.Thread(
                        target=_ensure_window_logic,
                        args=(title, min_w, min_h, intercept_close, 20.0),
                        daemon=True,
                    ).start()
                except Exception:  # noqa: BLE001
                    pass
        if _orig_wndproc.get(title):
            return user32.CallWindowProcW(_orig_wndproc[title], h, msg, wparam, lparam)
        return 0

    _wndproc_refs[title] = wndproc  # 防 GC 保活
    return wndproc


def _install_window_logic(title: str, min_w: int, min_h: int,
                          intercept_close: bool) -> bool:
    """把窗口逻辑完整交给 Windows 系统（官方自定义标题栏方案，同 Electron）。

    ⚠️ 窗口可能在 5s 兜底之后才创建完（WebView2 冷启动慢），此时 FindWindowW
    返回 0。置位必须在"成功打补丁之后"——否则首次失败会被永久记住，
    WndProc 不再安装 → 拖动/边缘缩放全部失效（表现为窗口拖不动）。
    """
    if _wnd_installed.get(title):
        return True
    _setup_user32()
    user32 = ctypes.windll.user32
    hwnd = user32.FindWindowW(None, title)
    if not hwnd:
        return False
    if title == APP_TITLE:
        gui_server.set_main_hwnd(int(hwnd))
    elif title == READER_TITLE:
        gui_server.set_reader_hwnd(int(hwnd))

    if title not in _orig_wndproc:
        _orig_wndproc[title] = 0

    GWL_STYLE = -16
    # frameless（无 WS_CAPTION / WS_BORDER）+ 自绘标题栏；thickframe 仅供
    # 边缘缩放手柄，min/max/sysmenu 仍保留供自绘按钮 / Alt+Space 菜单使用。
    # 沙箱 Edge 截图确认：frameless 真正干净后 .app 内容从 0,0 开始无横条。
    # （上一轮为了拿原生 min/max 补间动画加回 WS_CAPTION，但该位会让系统绘制
    # 标准标题栏——NCCALCSIZE return 0 只是把客户区撑到全窗，并不会阻止绘制。
    # 真机视觉上仍出现"系统栏+自绘栏"两层框，用户已反馈去此变化，回到 frameless。
    # ⚠️ Windows 样式位是 OR 累加的：pywebview frameless=True 会让 FormBorderStyle
    # 抑制绘制，但底层 style 仍含 WS_CAPTION 和 WS_BORDER；这里必须显式
    # AND ~WS_CAPTION / AND ~WS_BORDER 摘掉，否则系统栏 + DWM accent 细边
    # 会继续可见。
    WS_CAPTION = 0x00C00000
    WS_BORDER = 0x00800000
    WS_THICKFRAME, WS_MINIMIZEBOX = 0x00040000, 0x00020000
    WS_MAXIMIZEBOX, WS_SYSMENU = 0x00010000, 0x00080000
    style = user32.GetWindowLongPtrW(hwnd, GWL_STYLE)
    if style:
        style = ((style | WS_THICKFRAME | WS_MINIMIZEBOX
                  | WS_MAXIMIZEBOX | WS_SYSMENU)
                 & ~WS_CAPTION & ~WS_BORDER)
        user32.SetWindowLongPtrW(hwnd, GWL_STYLE, style)
    # DWM 防御：确保系统过渡动画未被禁用（DWMWA_TRANSITIONS_FORCEDISABLED = FALSE）
    try:
        dwm = ctypes.windll.dwmapi
        off = ctypes.c_int(0)
        dwm.DwmSetWindowAttribute(ctypes.c_void_p(int(hwnd)), 3,
                                  ctypes.byref(off), ctypes.sizeof(off))
    except Exception:  # noqa: BLE001 dwmapi 缺失不致命
        pass
    # 关闭非客户区渲染 → 系统不再绘制顶部 1px accent/白细框
    _apply_dwm(hwnd, rounded=True)
    proc_ptr = ctypes.cast(_make_wndproc(title, min_w, min_h, intercept_close),
                           ctypes.c_void_p).value
    prev = user32.SetWindowLongPtrW(hwnd, -4, proc_ptr)
    if not prev:
        # 子类化失败（返回 0）：不置 installed，允许轮询线程稍后重试。
        _log(f"[wndproc] {title} 子类化失败(SetWindowLongPtrW=0, "
             f"lastError={ctypes.get_last_error() or 'n/a'}) → 稍后重试")
        return False
    _orig_wndproc[title] = prev
    user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x4 | 0x1 | 0x2 | 0x20)  # FRAMECHANGED
    _wnd_installed[title] = True  # 仅在成功安装后记住，失败允许重试
    _caption_stripped.discard(title)   # WndProc 就绪 → 从降级态“升级”回带 caption
    _log(f"[wndproc] {title} WndProc 安装成功 hwnd={int(hwnd)}")
    return True


def _strip_caption(title: str) -> bool:
    """保底 no-op：主路径已 frameless（不 OR WS_CAPTION），无 caption 可摘。

    保留以备未来若回退加 WS_CAPTION 时能立即清理；当前 st & 0x00C00000 恒为 0。
    """
    try:
        _setup_user32()
        user32 = ctypes.windll.user32
        hwnd = user32.FindWindowW(None, title)
        if not hwnd:
            return False
        st = user32.GetWindowLongPtrW(hwnd, -16)
        if st & 0x00C00000:
            user32.SetWindowLongPtrW(hwnd, -16, st & ~0x00C00000)
            user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x4 | 0x1 | 0x2 | 0x20)
        _caption_stripped.add(title)
        if st & 0x00C00000:
            _log(f"[wndproc] {title} 降级：摘掉 WS_CAPTION（无原生动画，但绝无系统框）")
        return True
    except Exception:  # noqa: BLE001
        return False


def _ensure_window_logic(title: str, min_w: int, min_h: int,
                         intercept_close: bool, max_wait: float = 45.0) -> None:
    """轮询直到 WndProc 安装成功（窗口创建有迟滞，单次尝试不可靠）。

    超时仍未成功 → 调 _strip_caption 兜底（当前主路径已是 frameless，此路径
    通常 no-op，仅当未来某次回退加回 WS_CAPTION 时兜底摘除）。
    成功后启动 _guard_frameless 守护线程：pywebview 在 show / resize / focus
    等时机可能重新 set style 把 WS_CAPTION 加回来，需要持续维护。
    """
    deadline = _time.time() + max_wait
    while _time.time() < deadline:
        try:
            if _install_window_logic(title, min_w, min_h, intercept_close):
                threading.Thread(target=_guard_frameless, args=(title,),
                                 daemon=True, name=f"frameless-{title[:8]}").start()
                return
        except Exception:  # noqa: BLE001
            return
        _time.sleep(0.4)
    try:
        _strip_caption(title)
    except Exception:  # noqa: BLE001
        pass


def _guard_frameless(title: str) -> None:
    """frameless 守护线程：每 0.5s 扫一次主窗 + 阅读窗 GWL_STYLE，
    显式 `& ~WS_CAPTION` + `& ~WS_BORDER` 摘位。pywebview 在 show / resize /
    focus 等时机可能重新 set style 把 WS_CAPTION 和 WS_BORDER 加回来——
    无守护时会出现"系统栏+自绘栏"两层框 + DWM accent 1px 细边。
    窗口销毁时 FindWindowW 返回 0，安全退出。
    """
    _setup_user32()
    user32 = ctypes.windll.user32
    REMOVE_MASK = ~(0x00C00000 | 0x00800000)  # WS_CAPTION | WS_BORDER
    while True:
        try:
            hwnd = user32.FindWindowW(None, title)
            if hwnd:
                st = user32.GetWindowLongPtrW(int(hwnd), -16)
                if st and (st & 0x00C00000 or st & 0x00800000):
                    user32.SetWindowLongPtrW(int(hwnd), -16, st & REMOVE_MASK)
                    user32.SetWindowPos(int(hwnd), 0, 0, 0, 0, 0,
                                        0x4 | 0x1 | 0x2 | 0x20)  # FRAMECHANGED
                    _apply_dwm(int(hwnd), rounded=True)
        except Exception:  # noqa: BLE001
            pass
        _time.sleep(0.5)


# --------------------------------------------------------------------------- #
# 启动 / 关闭
# --------------------------------------------------------------------------- #
def _base_dir() -> Path:
    """应用基目录：frozen = exe 同目录；源码 = 仓库根（_MEIPASS 退出即清，不可用）。"""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent


_LOG_LOCK = threading.Lock()


def _log(msg: str) -> None:
    """窗口逻辑诊断日志（frozen windowed 无控制台，落 exe 同目录 logs/desktop.log）。"""
    try:
        with _LOG_LOCK:
            d = _base_dir() / "logs"
            d.mkdir(parents=True, exist_ok=True)
            with open(d / "desktop.log", "a", encoding="utf-8") as fh:
                fh.write(f"[{_time.strftime('%Y-%m-%d %H:%M:%S')}] {msg}\n")
    except Exception:  # noqa: BLE001 日志失败绝不影响主流程
        pass


_win_main: list = []
_win_reader: list = []
_server_ref: list = []
# 阅读窗当前打开的 (专辑, 章节) 元组（open-reader 幂等判断；同专辑换章必须重导航）
_reader_target: list = [None]
_reader_visible: list = [False]
_shown_once: list = [False]


def request_close() -> None:
    """主窗关闭按钮兜底：发 WM_CLOSE，1.5s 后若窗口仍在则强制退出。

    WinForms 宿主偶尔不响应 WM_CLOSE（尤其 WebView2 仍在初始化时），
    表现为"点关闭没反应"；这里保证点了就一定退。
    """
    try:
        hwnd = gui_server.get_main_hwnd()
        if hwnd:
            ctypes.windll.user32.PostMessageW(int(hwnd), WM_CLOSE, 0, 0)
    except Exception:  # noqa: BLE001
        pass

    def _force() -> None:
        _time.sleep(1.5)
        try:
            _setup_user32()
            ctypes.windll.user32.FindWindowW.restype = ctypes.c_void_p
            if ctypes.windll.user32.FindWindowW(None, APP_TITLE):
                _quit()
        except Exception:  # noqa: BLE001
            _quit()

    threading.Thread(target=_force, daemon=True).start()


def _quit() -> None:
    """统一退出路径：终止活动 worker → 停服务 → 立即退出。"""
    srv = _server_ref[0] if _server_ref else None
    try:
        gui_server.JOBS.shutdown_all()
    except Exception:  # noqa: BLE001
        pass
    if srv is not None:
        try:
            srv.shutdown()
        except Exception:  # noqa: BLE001
            pass
    os._exit(0)


# --------------------------------------------------------------------------- #
# reader（独立阅读窗）控制 —— 供 server.py window_action 调用
# --------------------------------------------------------------------------- #
def _show_reader_window() -> None:
    """把 reader 窗带到前台（GUI 线程无关：直接操作 hwnd，跨线程安全）。"""
    user32 = ctypes.windll.user32
    hwnd = gui_server.get_reader_hwnd()
    if not hwnd:
        return
    if user32.IsIconic(hwnd):
        user32.ShowWindow(hwnd, 9)  # SW_RESTORE
    # 级联定位：主窗右下偏移 72/56；越界回退主屏可用区居中
    try:
        main_h = gui_server.get_main_hwnd()
        mr = _RECT()
        if main_h and user32.GetWindowRect(int(main_h), ctypes.byref(mr)):
            sw = user32.GetSystemMetrics(0)
            sh = user32.GetSystemMetrics(1)
            x, y = mr.left + 72, mr.top + 56
            if x + READER_W > sw - 8 or y + READER_H > sh - 8:
                x, y = (sw - READER_W) // 2, (sh - READER_H) // 2
            user32.SetWindowPos(hwnd, 0, x, y, READER_W, READER_H, 0x4)
    except Exception:  # noqa: BLE001
        pass
    user32.ShowWindow(hwnd, 5)  # SW_SHOW
    user32.SetForegroundWindow(hwnd)
    _reader_visible[0] = True


def open_reader(album: str, title: str = "", chapter: str = "") -> dict:
    """主窗「在新窗口阅读」→ 显示/聚焦独立阅读窗。

    - 同专辑同章节已打开：仅聚焦（不重载，保留翻页进度/放大状态）；
    - 同专辑换章节或换专辑：导航到 ?w=reader&album=…[&ch=…] 后显示。
    由 HTTP worker 线程调用；操作全部走 hwnd 消息 + pywebview Window
    导航（pywebview 内部 marshaling 到 GUI 线程，线程安全）。
    """
    from urllib.parse import quote

    if not _win_reader:
        return {"ok": False, "error": "reader window not ready"}
    base = _server_ref[0].base_url if _server_ref and _server_ref[0] else "http://127.0.0.1"
    win = _win_reader[0]
    chapter = str(chapter or "").strip()
    target = (album, chapter) if chapter else (album, "")
    # 同专辑同章节已打开/曾被隐藏 → 直接显示复用（保留翻页进度与加载态，不重导航）
    if _reader_target[0] == target:
        _show_reader_window()
        return {"ok": True, "reused": True}

    url = f"{base}/?w=reader&album={quote(album)}"
    if title:
        url += f"&title={quote(title)}"
    if chapter:
        url += f"&ch={quote(chapter)}"
    try:
        win.load_url(url)
    except Exception as exc:  # noqa: BLE001 导航失败不致命（load_url 内部 marshaling）
        return {"ok": False, "error": str(exc)}
    _reader_target[0] = target
    # 等导航被接受后再显示（避免闪现旧内容）
    def _delayed_show():
        _time.sleep(0.25)
        _show_reader_window()

    threading.Thread(target=_delayed_show, daemon=True).start()
    return {"ok": True}


def close_reader() -> dict:
    """阅读窗关闭按钮 → 隐藏（窗口对象常驻复用，不退出应用）。"""
    hwnd = gui_server.get_reader_hwnd()
    if hwnd:
        ctypes.windll.user32.ShowWindow(int(hwnd), SW_HIDE)
    _reader_visible[0] = False
    return {"ok": True}


def toggle_reader_maximize() -> dict:
    hwnd = gui_server.get_reader_hwnd()
    if not hwnd:
        return {"ok": False, "error": "window not ready"}
    user32 = ctypes.windll.user32
    user32.PostMessageW(int(hwnd), WM_SYSCOMMAND,
                        SC_RESTORE if user32.IsZoomed(hwnd) else SC_MAXIMIZE, 0)
    return {"ok": True}


def minimize_reader() -> dict:
    hwnd = gui_server.get_reader_hwnd()
    if hwnd:
        ctypes.windll.user32.PostMessageW(int(hwnd), WM_SYSCOMMAND, 0xF020, 0)  # SC_MINIMIZE
    return {"ok": True}


# --------------------------------------------------------------------------- #
# main 窗口显隐（loaded / 兜底线程共用）
# --------------------------------------------------------------------------- #
def _show_window(force_center: bool = True) -> None:
    try:
        _setup_user32()
        hwnd = gui_server.get_main_hwnd()
        if hwnd:
            if force_center and not _shown_once[0]:
                sw = ctypes.windll.user32.GetSystemMetrics(0)
                sh = ctypes.windll.user32.GetSystemMetrics(1)
                x, y = (sw - WIDTH) // 2, (sh - HEIGHT) // 2
                ctypes.windll.user32.SetWindowPos(int(hwnd), 0, x, y, WIDTH, HEIGHT, 0x4)
                _shown_once[0] = True
            ctypes.windll.user32.ShowWindow(int(hwnd), 5)  # SW_SHOW
    except Exception:  # noqa: BLE001
        pass


def _frame_style(hwnd, *, fullscreen: bool) -> None:
    """按全屏状态应用窗口框架样式。

    frameless（无 WS_CAPTION / WS_BORDER）：常态仅 thickframe 供边缘缩放手柄；
    - fullscreen=True：临时摘掉 WS_THICKFRAME —— 系统不会再把窗口让出任务栏
      区域或画边框，由 SetWindowPos 直接铺满显示器（含任务栏区）；
    - fullscreen=False：恢复 thickframe（frameless 自绘栏态）。
    防御性显式 `& ~WS_CAPTION & ~WS_BORDER`：pywebview frameless=True 后底层
    style 仍可能含 caption + border，每次刷样式都摘掉。
    """
    _setup_user32()
    user32 = ctypes.windll.user32
    GWL_STYLE = -16
    WS_CAPTION, WS_BORDER = 0x00C00000, 0x00800000
    WS_THICKFRAME = 0x00040000
    WS_MINIMIZEBOX, WS_MAXIMIZEBOX, WS_SYSMENU = 0x00020000, 0x00010000, 0x00080000
    st = user32.GetWindowLongPtrW(hwnd, GWL_STYLE)
    if st:
        if fullscreen:
            st &= ~(WS_CAPTION | WS_BORDER | WS_THICKFRAME)
        else:
            st = ((st | WS_THICKFRAME | WS_MINIMIZEBOX
               | WS_MAXIMIZEBOX | WS_SYSMENU)
              & ~WS_CAPTION & ~WS_BORDER)
        user32.SetWindowLongPtrW(hwnd, GWL_STYLE, st)
        user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x4 | 0x1 | 0x2 | 0x20)  # FRAMECHANGED
    _apply_dwm(hwnd, rounded=not fullscreen)  # 全屏直角、常态圆角且无系统细框


def toggle_fullscreen(wid: str = "main") -> dict:
    """自建全屏（不走 pywebview toggle_fullscreen —— 它会把 FormBorderStyle 改成
    None + WindowState=Maximized，与我们的 NCCALCSIZE 的 rcWork 修正冲突，导致
    全屏盖不住任务栏）。

    进入：临时摘 caption/thickframe → 铺满所在显示器 rcMonitor（覆盖任务栏）；
    退出：恢复样式 → 还原原 rect（原为最大化则回到最大化）。切换由前端
    F11 / Esc / 阅读窗底栏「全屏」按钮触发（/api/window/toggle-fullscreen）。
    """
    try:
        _setup_user32()
    except Exception:  # noqa: BLE001
        pass
    user32 = ctypes.windll.user32
    title = APP_TITLE if wid != "reader" else READER_TITLE
    hwnd = gui_server.get_main_hwnd() if wid != "reader" else gui_server.get_reader_hwnd()
    if not hwnd:
        return {"ok": False, "error": "window not ready"}

    if _fullscreen.get(title):
        # ---- 退出全屏：恢复样式 + 还原窗口态 ----
        _fullscreen[title] = False
        try:
            _frame_style(hwnd, fullscreen=False)
        except Exception:  # noqa: BLE001
            pass
        box = _restore_box.pop(title, None)
        if box:
            left, top, right, bottom, was_max = box
            if was_max:
                user32.PostMessageW(int(hwnd), WM_SYSCOMMAND, SC_MAXIMIZE, 0)
            else:
                user32.SetWindowPos(int(hwnd), 0, left, top,
                                    right - left, bottom - top, 0x4 | 0x10)  # NOZORDER|NOACTIVATE
        user32.ShowWindow(int(hwnd), 5)  # SW_SHOW
        _log(f"[fullscreen] {title} 退出全屏")
        return {"ok": True, "fullscreen": False}

    # ---- 进入全屏 ----
    rr = _RECT()
    if not user32.GetWindowRect(int(hwnd), ctypes.byref(rr)):
        return {"ok": False, "error": "GetWindowRect failed"}
    was_max = bool(user32.IsZoomed(int(hwnd)))
    mon = user32.MonitorFromWindow(int(hwnd), 2)  # MONITOR_DEFAULTTONEAREST
    mi = MONITORINFO()
    mi.cbSize = ctypes.sizeof(MONITORINFO)
    if not user32.GetMonitorInfoW(mon, ctypes.byref(mi)):
        return {"ok": False, "error": "GetMonitorInfo failed"}
    _restore_box[title] = (rr.left, rr.top, rr.right, rr.bottom, was_max)
    _fullscreen[title] = True
    try:
        _frame_style(hwnd, fullscreen=True)
    except Exception:  # noqa: BLE001
        pass
    m = mi.rcMonitor
    user32.SetWindowPos(int(hwnd), 0, m.left, m.top,
                        m.right - m.left, m.bottom - m.top, 0x20 | 0x40)  # FRAMECHANGED|SHOWWINDOW
    user32.SetForegroundWindow(int(hwnd))
    _log(f"[fullscreen] {title} 进入全屏 {m.right - m.left}x{m.bottom - m.top}")
    return {"ok": True, "fullscreen": True}


def fullscreen_state(wid: str = "main") -> bool:
    """查询窗口当前是否处于自建全屏态（供 is-maximized 等接口同步前端图标）。"""
    title = APP_TITLE if wid != "reader" else READER_TITLE
    return bool(_fullscreen.get(title))


def wndproc_status(wid: str = "main") -> dict:
    """诊断接口：WndProc 安装/降级/全屏/拖动模式（配合 logs/desktop.log 排查）。"""
    title = APP_TITLE if wid != "reader" else READER_TITLE
    return {
        "ok": True,
        "title": title,
        "installed": bool(_wnd_installed.get(title)),
        "dead": bool((_state_box.get(title) or {}).get("dead")),
        "captionStripped": title in _caption_stripped,
        "fullscreen": bool(_fullscreen.get(title)),
        "nativeDrag": _NATIVE_DRAG,
    }


def main(server: gui_server.GuiServer, dev: bool = False) -> None:
    _server_ref.append(server)
    webview.settings["DRAG_REGION_SELECTOR"] = ".pywebview-drag-region-disabled"
    webview.settings["DRAG_REGION_DIRECT_TARGET_ONLY"] = True

    # WebView2 数据隔离：应用专属缓存目录，避免与系统其它 WebView2 共享
    cache_dir = str(_base_dir() / ".webview2-cache")
    os.makedirs(cache_dir, exist_ok=True)

    def _on_main_loaded():
        _show_window(force_center=True)
        _ensure_window_logic(APP_TITLE, MIN_W, MIN_H, intercept_close=False)

    def _on_reader_loaded():
        # reader 窗保持隐藏，仅确保 WndProc 就绪（WM_CLOSE→隐藏等逻辑）
        _ensure_window_logic(READER_TITLE, READER_MIN_W, READER_MIN_H, intercept_close=True)

    # 两个窗口都须在 start() 前创建（pywebview 6 不支持运行期新增窗口）
    main_win = webview.create_window(
        APP_TITLE,
        server.base_url,
        width=WIDTH,
        height=HEIGHT,
        min_size=(MIN_W, MIN_H),
        frameless=True,
        easy_drag=False,       # 拖动交 WM_NCHITTEST → 系统原生
        text_select=True,
        js_api=Api(server),
        background_color="#f7f4ee",
        confirm_close=False,
        hidden=True,           # 先隐藏，loaded 后再显示，避免 WebView2 容器闪现
    )
    _win_main.append(main_win)
    reader_win = webview.create_window(
        READER_TITLE,
        server.base_url + "/?w=reader",
        width=READER_W,
        height=READER_H,
        min_size=(READER_MIN_W, READER_MIN_H),
        frameless=True,
        easy_drag=False,
        text_select=True,
        js_api=Api(server),
        background_color="#f7f4ee",
        confirm_close=False,
        hidden=True,
    )
    _win_reader.append(reader_win)

    main_win.events.loaded += _on_main_loaded
    reader_win.events.loaded += _on_reader_loaded
    main_win.events.closed += lambda: _quit()

    def _fallback():
        _time.sleep(5)
        _show_window(force_center=True)
        _ensure_window_logic(APP_TITLE, MIN_W, MIN_H, intercept_close=False)
        _ensure_window_logic(READER_TITLE, READER_MIN_W, READER_MIN_H, intercept_close=True)

    # 窗口晚建（WebView2 冷启动）时持续重试，直到 WndProc 安装成功
    threading.Thread(target=_fallback, daemon=True).start()
    threading.Thread(
        target=_ensure_window_logic, args=(APP_TITLE, MIN_W, MIN_H, False), daemon=True
    ).start()
    threading.Thread(
        target=_ensure_window_logic, args=(READER_TITLE, READER_MIN_W, READER_MIN_H, True),
        daemon=True,
    ).start()

    def _watchdog():
        seen = False
        while True:
            try:
                _time.sleep(1)
                _setup_user32()
                ctypes.windll.user32.FindWindowW.restype = ctypes.c_void_p
                if ctypes.windll.user32.FindWindowW(None, APP_TITLE):
                    seen = True
                elif seen:
                    _quit()
            except Exception:  # noqa: BLE001
                pass

    threading.Thread(target=_watchdog, daemon=True).start()

    try:
        webview.start(debug=dev, private_mode=False, storage_path=cache_dir)
        _quit()
    except Exception as exc:  # noqa: BLE001 无 GUI 环境 → 降级浏览器模式
        if dev:
            print(f"[desktop] 桌面窗口启动失败({exc})，降级为浏览器模式", flush=True)
        import webbrowser

        webbrowser.open(server.base_url)
        try:
            server.httpd.serve_forever()
        except KeyboardInterrupt:
            pass
