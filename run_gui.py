# -*- coding: utf-8 -*-
"""JMComic Downloader GUI · 桌面入口

用法：
    python run_gui.py            # 启动本地服务 + 桌面窗口
    python run_gui.py --dev      # 开发模式：窗口 debug + 控制台日志
    python run_gui.py --browser  # 仅启动本地服务并在系统浏览器打开（无 GUI 环境调试）

Windows 控制台隐藏由 PyInstaller --windowed 处理；脚本方式运行会保留控制台。
"""
from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# 单 exe 分发：主程序内嵌下载 worker 入口。
# PyInstaller onefile 打包后，下载子进程由 jobs.py 以 `本exe --jm-worker <worker参数>`
# 方式自我拉起（无需同目录 Worker.exe）。须在 import gui.server（及其 Qt/服务依赖）之前
# 拦截分派，让 worker 进程以最小开销启动。
if len(sys.argv) > 1 and sys.argv[1] == "--jm-worker":
    sys.argv.pop(1)
    from gui import worker  # noqa: E402
    raise SystemExit(worker.main())

from gui import server as gui_server  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("--dev", action="store_true", help="开发模式（窗口 debug + 服务日志）")
    parser.add_argument("--browser", action="store_true", help="仅启动本地服务并在浏览器打开")
    parser.add_argument("--port", type=int, default=0, help="本地服务端口（默认随机空闲端口）")
    args = parser.parse_args()

    srv = gui_server.create_server(args.port)
    srv.start()
    print(f"[gui] local server → {srv.base_url}", flush=True)

    if args.browser:
        import webbrowser
        webbrowser.open(srv.base_url)
        try:
            srv.httpd.serve_forever()
        except KeyboardInterrupt:
            pass
        return 0

    # 桌面窗口（pywebview + WebView2 双窗口壳：主窗 + 独立阅读窗）
    from gui import desktop
    try:
        desktop.main(srv, dev=args.dev)
    except SystemExit:
        raise
    except BaseException:  # noqa: BLE001  windowed 无控制台，崩溃落盘便于诊断
        import traceback
        try:
            base = Path(sys.executable if getattr(sys, "frozen", False) else __file__).resolve().parent
            (base / "logs").mkdir(exist_ok=True)
            with open(base / "logs" / "gui-crash.log", "w", encoding="utf-8") as f:
                f.write(traceback.format_exc())
        except Exception:  # noqa: BLE001
            pass
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
