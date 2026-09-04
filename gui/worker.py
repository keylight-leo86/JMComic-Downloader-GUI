# -*- coding: utf-8 -*-
"""下载任务子进程入口。

GUI 服务以 Popen 方式拉起 `python -m gui.worker --kind ... --ids ... --log-file ...`。
子进程复用 `jmcomic_gui.run_worker` 完成实际下载与 PDF 整合；本文件只负责：
  1. CLI 参数解析；
  2. 把 `run_worker` 写入日志文件的内容**逐行**转发到 stdout（JSON-line 事件 `{"type":"log",...}`）；
  3. 收尾时再追加一条 `{"type":"status",...}`，状态机：`done` (rc=0) / `failed` (非 0)。

⚠️ 事件通道与退出时序有三处硬约束（均有实测教训，勿回退）：
  1. `jmcomic_gui.run_worker` 会把 sys.stdout/sys.stderr 重定向到日志文件，
     若事件走 sys.stdout 写入会造成"事件行→日志→再读取→再写"的指数嵌套回环
     （曾实测 9.5s 膨胀 536MB）→ 事件必须直写进程启动时的原始 stdout fd。
  2. Windows 匿名管道写可能阻塞（读端停读/缓冲打满），主线程直接写会被卡死、
     进程无法退出、GUI 侧 proc.wait() 永不返回 → 事件写入统一交给 daemon 写线程。
  3. PyInstaller 打包版常规退出会被第三方库遗留的非 daemon 后台线程拖住数十秒
     （实测 36~90s+）→ 终态事件入队后直接 os._exit(rc)。
"""
from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import threading
import time
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

import jmcomic_gui  # noqa: E402

# 事件通道 = 进程启动时的真实 stdout fd。
# 注意：jmcomic_gui.run_worker 会把 sys.stdout/sys.stderr 重定向到 --log-file，
# 若 _emit 写 sys.stdout，事件行会被追加进日志文件 → tail 线程读到后再次 _emit，
# 形成指数级嵌套回环（每行 JSON 层层包裹，秒级膨胀到数百 MB）。
# 因此事件通道固定直写原始 fd，彻底绕过 sys.stdout 替换。
_EVENT_FD: int | None = None
try:
    _EVENT_FD = sys.stdout.fileno()
except (OSError, ValueError, AttributeError):
    _EVENT_FD = None

# 所有事件写入统一交给后台 daemon 写线程消费队列：
# Windows 匿名管道在极端情况下 WriteFile 可能阻塞（如 GUI 读端短暂停读、
# 缓冲打满），若主线程直接 os.write 会被永久卡住，导致进程无法退出、
# GUI 侧 proc.wait() 永不返回、任务永远停在 running。
# 让写线程承担阻塞风险（daemon 线程，os._exit 时直接终止），
# 主线程的 _emit 只做入队，保证任何情况下进程都能及时退出。
_EVENT_QUEUE: "queue.Queue[dict]" = queue.Queue(maxsize=10000)


def _event_writer_loop() -> None:
    global _EVENT_FD
    while True:
        ev = _EVENT_QUEUE.get()
        line = (json.dumps(ev, ensure_ascii=False) + "\n").encode("utf-8", "replace")
        if _EVENT_FD is not None:
            try:
                os.write(_EVENT_FD, line)
                continue
            except OSError:
                _EVENT_FD = None  # 管道已断 → 回落 sys.stdout，静默容错
        try:
            sys.stdout.write(line.decode("utf-8", "replace"))
            sys.stdout.flush()
        except (OSError, ValueError):
            pass


def _emit(ev: dict) -> None:
    try:
        _EVENT_QUEUE.put_nowait(ev)
    except Exception:
        pass  # 队列满则丢弃事件（进度可视化缺失，但不影响任务终态收敛）


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="gui.worker")
    parser.add_argument("--kind", required=True, choices=["album", "photo"])
    parser.add_argument("--ids", nargs="+", required=True, help="车号或章节 ID 列表")
    parser.add_argument("--output", required=True, help="保存位置（绝对路径）")
    parser.add_argument("--client", default="api", choices=["api", "html"], help="客户端实现")
    parser.add_argument("--proxy", default="", help="代理（留空=跟随系统）")
    parser.add_argument("--image-format", default="original", help="图片格式（original/jpg/png/webp...）")
    parser.add_argument("--image-threads", type=int, default=20)
    parser.add_argument("--photo-threads", type=int, default=4)
    parser.add_argument("--option", default="", help="高级 YAML 配置文件路径（可选）")
    parser.add_argument("--log-file", required=True, help="run_worker 写入的日志文件路径")
    return parser


def _tail_log_to_stdout(path: Path, stop_event: threading.Event) -> None:
    while not stop_event.is_set() and not path.exists():
        time.sleep(0.05)
    if stop_event.is_set():
        return
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            while not stop_event.is_set():
                line = fh.readline()
                if line:
                    _forward_line(line.rstrip("\n"))
                else:
                    if stop_event.is_set():
                        break
                    time.sleep(0.08)
    except OSError as exc:
        _emit({"type": "log", "line": f"[worker] 日志读取失败：{exc}", "level": "error"})


def _forward_line(line: str) -> None:
    """把日志文件单行转成事件：@@PROGRESS@@ 前缀 → progress 事件，其余 → log 事件。"""
    if line.startswith("@@PROGRESS@@"):
        payload = line[len("@@PROGRESS@@"):].strip()
        try:
            data = json.loads(payload)
        except ValueError:
            return
        _emit({"type": "progress", "data": data})
        return
    _emit({"type": "log", "line": line})


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    args = parser.parse_args(argv)

    log_path = Path(args.log_file)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    if log_path.exists():
        try:
            log_path.unlink()
        except OSError:
            pass

    ns = argparse.Namespace(
        kind=args.kind,
        ids=args.ids,
        output=args.output,
        client=args.client,
        proxy=args.proxy,
        image_format=args.image_format,
        image_threads=args.image_threads,
        photo_threads=args.photo_threads,
        option=args.option or None,
        log_file=str(log_path),
    )

    stop_event = threading.Event()
    threading.Thread(target=_event_writer_loop, name="event-writer", daemon=True).start()
    tail_thread = threading.Thread(target=_tail_log_to_stdout, args=(log_path, stop_event), daemon=True)
    tail_thread.start()

    _emit({"type": "status", "state": "running", "log": str(log_path)})

    rc = 1
    try:
        rc = jmcomic_gui.run_worker(ns)
    except SystemExit as exc:
        rc = int(exc.code or 1)
    except Exception:
        _emit({"type": "log", "line": traceback.format_exc().rstrip(), "level": "error"})
        rc = 1
    finally:
        time.sleep(0.25)
        stop_event.set()
        tail_thread.join(timeout=1.0)

    state = "done" if rc == 0 else "failed"
    _emit({"type": "status", "state": state, "returncode": rc})
    # PyInstaller 打包环境下，解释器常规退出（SystemExit → threading._shutdown）
    # 可能被第三方库遗留的非 daemon 后台线程拖住数十秒（实测 36~90s+，疑似
    # curl_cffi 等网络连接清理）。此时状态事件已入队、日志已 flush/close，
    # 下载产物均已落盘，直接 os._exit 确保进程立即终止，让 GUI 侧 proc.wait()
    # 及时收敛任务终态。源码模式保留常规退出路径，便于调试与测试。
    if getattr(sys, "frozen", False):
        os._exit(rc)
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
