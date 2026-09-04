# -*- coding: utf-8 -*-
"""下载任务管理：维护 job_id → 子进程映射、订阅事件流、限流并发。

事件协议（与 gui/worker.py 输出对齐）：
  {"type": "log", "line": str, "level"?: "error"}
  {"type": "status", "state": "running"|"done"|"failed"|"cancelled", "returncode"?: int, ...}
"""
from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Deque, Optional


TERMINAL_STATES = {"done", "failed", "cancelled"}

# PyInstaller 打包版：下载子进程优先使用与主 GUI exe 同目录的独立 Worker.exe
# （console 子系统，启动开销小；GUI 以 CREATE_NO_WINDOW 拉起，避免弹黑窗）。
# 若同目录不存在 Worker.exe（单 exe 分发），则回退为「本 exe --jm-worker」自我拉起：
# run_gui.py 入口在加载 Qt/服务前拦截该参数并转交 gui.worker.main()（见 run_gui.py 顶部）。
# 未打包的源码模式仍用 python + gui/worker.py。
WORKER_EXE_NAME = "JMComic-Downloader-Worker.exe"


@dataclass
class JobOptions:
    kind: str                       # "album" | "photo"
    ids: list[str]
    output_dir: str
    client: str = "api"
    proxy: str = ""
    image_format: str = "original"
    image_threads: int = 20
    photo_threads: int = 4
    option_file: str = ""

    def to_argv(self) -> list[str]:
        argv = [
            "--kind", self.kind,
            "--ids", *self.ids,
            "--output", self.output_dir,
            "--client", self.client,
            "--proxy", self.proxy,
            "--image-format", self.image_format,
            "--image-threads", str(self.image_threads),
            "--photo-threads", str(self.photo_threads),
        ]
        if self.option_file:
            argv += ["--option", self.option_file]
        return argv


@dataclass
class JobRecord:
    job_id: str
    options: JobOptions
    state: str = "queued"           # queued | running | done | failed | cancelled
    returncode: Optional[int] = None
    events: Deque[dict] = field(default_factory=lambda: deque(maxlen=4000))
    subscribers: list[queue.Queue] = field(default_factory=list)
    cancel_requested: bool = False
    process: Optional[subprocess.Popen] = None
    submitted_at: float = 0.0
    started_at: float = 0.0
    finished_at: float = 0.0
    summary: dict = field(default_factory=dict)
    log_path: str = ""
    progress: Optional[dict] = None   # 最近一次 @@PROGRESS@@ 事件数据（下载/PDF 阶段）

    # 前端「一键重下/重试」可直接回读 options 字段再 POST /api/jobs
    def to_options_dict(self) -> dict:
        o = self.options
        return {
            "outputDir": o.output_dir,
            "client": o.client,
            "proxy": o.proxy,
            "imageFormat": o.image_format,
            "imageThreads": o.image_threads,
            "photoThreads": o.photo_threads,
            "optionFile": o.option_file,
        }

    def to_summary(self) -> dict:
        return {
            "job_id": self.job_id,
            "state": self.state,
            "kind": self.options.kind,
            "ids": self.options.ids,
            "output": self.options.output_dir,
            "options": self.to_options_dict(),
            "submitted_at": self.submitted_at,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "returncode": self.returncode,
            "cancel_requested": self.cancel_requested,
            "log": self.log_path,
            "summary": self.summary,
            "progress": self.progress,
        }


class JobManager:
    def __init__(self, *, root: Path, max_concurrent: int = 2,
                 on_event: Optional[Callable[[str, dict], None]] = None,
                 history_path: Optional[str] = None,
                 history_limit: int = 200):
        self._root = root
        self._jobs: dict[str, JobRecord] = {}
        self._lock = threading.Lock()
        self._semaphore = threading.Semaphore(max(1, max_concurrent))
        self._max_concurrent = max(1, max_concurrent)
        self._on_event = on_event or (lambda jid, ev: None)
        # 日志目录：源码模式跟随仓库根；打包模式改放 exe 同目录（_MEIPASS 是临时
        # 解包目录，退出即被清理，日志落在那里会丢失）。
        self._logs_dir = self._root / "logs"
        if getattr(sys, "frozen", False):
            try:
                self._logs_dir = Path(sys.executable).resolve().parent / "logs"
            except Exception:
                pass
        self._logs_dir.mkdir(parents=True, exist_ok=True)
        # 历史记录：终态任务落盘（history_path 默认跟随系统数据目录，便于重启后展示）
        self._history: Deque[dict] = deque(maxlen=max(1, history_limit))
        self._history_path: Optional[Path] = None
        if history_path:
            self._history_path = Path(history_path)
            self._history_path.parent.mkdir(parents=True, exist_ok=True)
            self._load_history()
            self._seed_jobs_from_history()

    # ---------------- 历史持久化 ----------------
    def _load_history(self) -> None:
        try:
            raw = json.loads(self._history_path.read_text(encoding="utf-8"))
        except (OSError, ValueError, TypeError):
            return
        if not isinstance(raw, list):
            return
        for item in raw:
            if isinstance(item, dict) and item.get("job_id"):
                self._history.append(item)

    def _seed_jobs_from_history(self) -> None:
        """重启后把历史终态任务回填到下载视图（只读记录，无子进程）。"""
        for item in self._history:
            jid = item.get("job_id")
            if not jid or jid in self._jobs:
                continue
            state = item.get("state")
            if state not in TERMINAL_STATES:
                continue
            opts = item.get("options") or {}
            options = JobOptions(
                kind=item.get("kind") or "album",
                ids=item.get("ids") or [],
                output_dir=item.get("output") or opts.get("outputDir") or str(self._root),
                client=opts.get("client") or "api",
                proxy=opts.get("proxy") or "",
                image_format=opts.get("imageFormat") or "original",
                image_threads=int(opts.get("imageThreads") or 20),
                photo_threads=int(opts.get("photoThreads") or 4),
                option_file=opts.get("optionFile") or "",
            )
            rec = JobRecord(job_id=jid, options=options, state=state,
                            log_path=item.get("log") or str(self._logs_dir / f"{jid}.log"))
            rec.returncode = item.get("returncode")
            rec.summary = item.get("summary") or {}
            rec.submitted_at = item.get("submitted_at") or 0.0
            rec.started_at = item.get("started_at") or 0.0
            rec.finished_at = item.get("finished_at") or 0.0
            rec.progress = item.get("progress") or None
            self._jobs[jid] = rec

    def _persist_history(self) -> None:
        if not self._history_path:
            return
        try:
            tmp = self._history_path.with_suffix(".json.tmp")
            tmp.write_text(json.dumps(list(self._history), ensure_ascii=False, indent=1),
                           encoding="utf-8")
            tmp.replace(self._history_path)
        except OSError:
            pass

    def history(self) -> list[dict]:
        with self._lock:
            return list(reversed(self._history))  # 最新在前

    def clear_history(self) -> int:
        with self._lock:
            n = len(self._history)
            self._history.clear()
        self._persist_history()
        return n

    def remove_history(self, job_id: str) -> bool:
        """从历史记录中移除单条（下载历史视图的删除）。"""
        with self._lock:
            before = len(self._history)
            self._history = deque(
                (item for item in self._history if item.get("job_id") != job_id),
                maxlen=self._history.maxlen,
            )
            removed = len(self._history) != before
        if removed:
            self._persist_history()
        return removed

    # ---------------- 公共接口 ----------------
    def submit(self, options: JobOptions) -> JobRecord:
        if options.kind not in ("album", "photo"):
            raise ValueError(f"不支持的下载类型：{options.kind}")
        if not options.ids:
            raise ValueError("ids 不能为空")
        if not options.output_dir:
            raise ValueError("output_dir 不能为空")
        job_id = uuid.uuid4().hex[:12]
        rec = JobRecord(job_id=job_id, options=options,
                        log_path=str(self._logs_dir / f"{job_id}.log"))
        rec.submitted_at = time.time()
        with self._lock:
            self._jobs[job_id] = rec
        threading.Thread(target=self._run, args=(rec,), daemon=True).start()
        return rec

    def list(self) -> list[dict]:
        with self._lock:
            out = []
            queued_seen = 0
            for j in self._jobs.values():
                s = j.to_summary()
                if j.state == "queued":
                    queued_seen += 1
                    s["queuePos"] = queued_seen
                out.append(s)
            return out

    def stats(self) -> dict:
        """当前队列视图：并发上限、进行中、排队、总数。"""
        with self._lock:
            running = queued = 0
            for j in self._jobs.values():
                if j.state == "running":
                    running += 1
                elif j.state == "queued":
                    queued += 1
            return {
                "maxConcurrent": self._max_concurrent,
                "running": running,
                "queued": queued,
                "total": len(self._jobs),
            }

    def get(self, job_id: str) -> Optional[JobRecord]:
        with self._lock:
            return self._jobs.get(job_id)

    def cancel(self, job_id: str) -> bool:
        with self._lock:
            rec = self._jobs.get(job_id)
        if not rec:
            return False
        rec.cancel_requested = True
        proc = rec.process
        if proc and proc.poll() is None:
            try:
                proc.terminate()
            except Exception:
                pass
        return True

    def shutdown_all(self) -> None:
        """应用退出前终止所有未完成任务。

        防两类问题：① 孤儿 worker 在无 UI 状态下继续下载；② frozen 下 worker
        进程持住主 exe 临时解包目录（_MEIPASS）句柄 → 主程序退出时 PyInstaller
        删不掉临时目录并弹告警对话框。
        """
        with self._lock:
            active = [r for r in self._jobs.values() if r.state not in TERMINAL_STATES]
        for rec in active:
            try:
                self.cancel(rec.job_id)
            except Exception:  # noqa: BLE001 退出路径尽力而为
                pass

    def remove(self, job_id: str) -> bool:
        """从活动列表移除（终态任务清理）；不影响历史记录。"""
        with self._lock:
            rec = self._jobs.get(job_id)
            if rec is None:
                return False
            if rec.state in ("queued", "running"):
                return False  # 活动任务只能取消，不能直接移除
            del self._jobs[job_id]
        return True

    def _remember_terminal(self, rec: JobRecord) -> None:
        """终态任务写进历史（供下载历史视图 + 重启后恢复展示）。"""
        with self._lock:
            summary = rec.to_summary()
            for existing in self._history:
                if existing.get("job_id") == rec.job_id:
                    existing.update(summary)
                    break
            else:
                self._history.append(summary)
        self._persist_history()

    # ---------------- 内部 ----------------
    def _emit(self, rec: JobRecord, ev: dict) -> None:
        rec.events.append(ev)
        for q in list(rec.subscribers):
            try:
                q.put_nowait(ev)
            except Exception:
                pass
        try:
            self._on_event(rec.job_id, ev)
        except Exception:
            pass

    def _run(self, rec: JobRecord) -> None:
        with self._semaphore:
            if rec.cancel_requested:
                rec.state = "cancelled"
                rec.finished_at = time.time()
                self._emit(rec, {"type": "status", "state": "cancelled"})
                self._remember_terminal(rec)
                return

            output_dir = rec.options.output_dir
            try:
                Path(output_dir).mkdir(parents=True, exist_ok=True)
            except OSError as exc:
                rec.state = "failed"
                rec.finished_at = time.time()
                rec.summary = {"error": str(exc)}
                self._emit(rec, {"type": "status", "state": "failed", "error": str(exc)})
                self._remember_terminal(rec)
                return

            env = os.environ.copy()
            env["PYTHONIOENCODING"] = "utf-8"
            env["PYTHONUTF8"] = "1"

            # 子进程命令：
            # - 测试钩子：JM_WORKER_OVERRIDE 指向独立 worker 脚本（offline 烟测，仅源码模式）
            # - 打包模式：同目录 JMComic-Downloader-Worker.exe（CREATE_NO_WINDOW 防黑窗）
            #             若无该 exe（单 exe 分发）→ 本 exe --jm-worker 自我拉起（见 run_gui.py 分派）
            # - 源码模式：python gui/worker.py
            override = os.environ.get("JM_WORKER_OVERRIDE")
            frozen = getattr(sys, "frozen", False)
            pop_kwargs: dict = {}
            if frozen:
                worker_exe = Path(sys.executable).with_name(WORKER_EXE_NAME)
                worker_cmd = [str(worker_exe)] if worker_exe.is_file() else [sys.executable, "--jm-worker"]
                cmd = [*worker_cmd, *rec.options.to_argv(), "--log-file", rec.log_path]
                if os.name == "nt":
                    pop_kwargs["creationflags"] = 0x08000000  # CREATE_NO_WINDOW
            elif override:
                cmd = [sys.executable, override, *rec.options.to_argv(),
                       "--log-file", rec.log_path]
            else:
                cmd = [sys.executable, str(self._root / "gui" / "worker.py"),
                       *rec.options.to_argv(), "--log-file", rec.log_path]
            try:
                proc = subprocess.Popen(
                    cmd, cwd=str(self._root), env=env,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    # 显式 UTF-8：worker 事件流恒为 UTF-8（子进程侧已设 PYTHONUTF8=1），
                    # 若不指定，frozen GUI 在中文 Windows 下默认 GBK 解码 → UnicodeDecodeError
                    # 使 _drain_stream 线程崩溃、任务终态永不收敛（冻结冒烟实测）。
                    text=True, encoding="utf-8", errors="replace", bufsize=1, **pop_kwargs,
                )
            except Exception as exc:
                rec.state = "failed"
                rec.finished_at = time.time()
                rec.summary = {"error": str(exc)}
                self._emit(rec, {"type": "status", "state": "failed", "error": str(exc)})
                self._remember_terminal(rec)
                return

            rec.process = proc
            rec.state = "running"
            rec.started_at = time.time()
            self._emit(rec, {"type": "status", "state": "running", "pid": proc.pid})

            out_t = threading.Thread(target=self._drain_stream, args=(proc.stdout, rec), daemon=True)
            err_t = threading.Thread(target=self._drain_stream, args=(proc.stderr, rec), daemon=True)
            out_t.start()
            err_t.start()

            try:
                rc = proc.wait()
            except Exception:
                rc = -1

            out_t.join(timeout=2.0)
            err_t.join(timeout=2.0)
            rec.returncode = rc
            rec.finished_at = time.time()
            if rec.cancel_requested:
                rec.state = "cancelled"
                rec.summary = {"cancelled": True}
            elif rc == 0:
                rec.state = "done"
                rec.summary = {"ok": True}
            else:
                rec.state = "failed"
                rec.summary = {"ok": False, "returncode": rc}
            self._emit(rec, {"type": "status", "state": rec.state, "returncode": rc})
            self._remember_terminal(rec)

    def _drain_stream(self, stream, rec: JobRecord) -> None:
        for raw in iter(stream.readline, ""):
            line = raw.rstrip("\n")
            if not line:
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                ev = {"type": "log", "line": line, "level": "error"}
            if ev.get("type") == "progress" and isinstance(ev.get("data"), dict):
                rec.progress = ev["data"]  # 保持最新进度，供列表/重连读取
            # 终态事件需立即同步到记录状态，避免 SSE 早于 _run 主线程的状态赋值
            if ev.get("type") == "status" and ev.get("state") in TERMINAL_STATES:
                if rec.state not in TERMINAL_STATES:
                    rec.state = ev["state"]
                if "returncode" in ev and rec.returncode is None:
                    rec.returncode = ev["returncode"]
                if not rec.finished_at:
                    rec.finished_at = time.time()
                # 历史同步落盘：状态对查询可见时历史文件必须已存在
                # （_run 主线程稍后还会再调一次，_remember_terminal 幂等无害）
                self._remember_terminal(rec)
            self._emit(rec, ev)
        try:
            stream.close()
        except Exception:
            pass
