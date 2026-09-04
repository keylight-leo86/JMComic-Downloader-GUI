# -*- coding: utf-8 -*-
"""gui.server 任务管理相关路由的离线测试。

不真实启动子进程，注入一个桩 JobManager（手工构造 JobRecord、模拟事件流），
再对真实 HTTP 服务发起请求，验证：
  - JobOptions.to_argv / JobManager.submit 校验
  - POST /api/jobs 输入校验与提交返回
  - GET  /api/jobs 列表
  - GET  /api/jobs/<id> 详情/404
  - DELETE /api/jobs/<id> 取消
  - GET  /api/jobs/<id>/events SSE：先回放历史，状态终态后连接关闭
"""
from __future__ import annotations

import json
import os
import socket
import time
import unittest
import uuid
from pathlib import Path
from urllib import request as urlrequest

from gui import jobs as gui_jobs
from gui import server as gui_server


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #

def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _http(method: str, url: str, body: dict | None = None, headers: dict | None = None,
          timeout: float = 8.0) -> tuple[int, dict, bytes]:
    raw = b"" if body is None else json.dumps(body).encode("utf-8")
    req = urlrequest.Request(url, data=raw if body is not None else None, method=method,
                             headers={"Content-Type": "application/json", **(headers or {})})
    try:
        with urlrequest.urlopen(req, timeout=timeout) as resp:
            return resp.status, dict(resp.headers), resp.read()
    except urlrequest.HTTPError as exc:
        return exc.code, dict(exc.headers or {}), exc.read() or b""


class _StubManager:
    """最小可用的 manager 替身，绕过真实 Popen 路径。"""

    def __init__(self):
        self.records: dict[str, gui_jobs.JobRecord] = {}
        self._history: list[dict] = []

    def submit(self, options: gui_jobs.JobOptions) -> gui_jobs.JobRecord:
        from gui.jobs import JobRecord
        rec = JobRecord(job_id=uuid.uuid4().hex[:12], options=options)
        rec.log_path = f"logs/{rec.job_id}.log"
        rec.state = "queued"
        rec.submitted_at = time.time()
        self.records[rec.job_id] = rec
        return rec

    def list(self):
        return [r.to_summary() for r in self.records.values()]

    def stats(self):
        running = sum(1 for r in self.records.values() if r.state == "running")
        queued = sum(1 for r in self.records.values() if r.state == "queued")
        return {"maxConcurrent": 2, "running": running, "queued": queued,
                "total": len(self.records)}

    def history(self):
        return list(reversed(self._history))

    def clear_history(self) -> int:
        n = len(self._history)
        self._history.clear()
        return n

    def remove_history(self, job_id: str) -> bool:
        before = len(self._history)
        self._history = [h for h in self._history if h.get("job_id") != job_id]
        return len(self._history) != before

    def get(self, jid: str):
        return self.records.get(jid)

    def remove(self, jid: str) -> bool:
        rec = self.records.get(jid)
        if rec is None or rec.state in ("queued", "running"):
            return False
        del self.records[jid]
        return True

    def cancel(self, jid: str) -> bool:
        rec = self.records.get(jid)
        if not rec:
            return False
        rec.cancel_requested = True
        rec.state = "cancelled"
        rec.finished_at = time.time()
        rec.events.append({"type": "status", "state": "cancelled"})
        for q in list(rec.subscribers):
            try:
                q.put_nowait({"type": "status", "state": "cancelled"})
            except Exception:
                pass
        return True


# --------------------------------------------------------------------------- #
# 单元
# --------------------------------------------------------------------------- #

class JobOptionsTest(unittest.TestCase):
    def test_to_argv_basic(self):
        o = gui_jobs.JobOptions(
            kind="album", ids=["1001", "1002"], output_dir="C:/d",
            client="api", proxy="", image_format="original",
            image_threads=20, photo_threads=4,
        )
        argv = o.to_argv()
        self.assertIn("--kind", argv); self.assertIn("album", argv)
        self.assertIn("--ids", argv); self.assertIn("1001", argv); self.assertIn("1002", argv)
        self.assertIn("--output", argv); self.assertIn("C:/d", argv)
        self.assertNotIn("--option", argv)

    def test_to_argv_with_option(self):
        o = gui_jobs.JobOptions(kind="photo", ids=["x"], output_dir="d",
                                option_file="adv.yml")
        argv = o.to_argv()
        self.assertIn("--option", argv); self.assertIn("adv.yml", argv)

    def test_manager_validates(self):
        m = gui_jobs.JobManager(root=Path.cwd(), max_concurrent=1)
        with self.assertRaises(ValueError):
            m.submit(gui_jobs.JobOptions(kind="wrong", ids=["a"], output_dir="d"))
        with self.assertRaises(ValueError):
            m.submit(gui_jobs.JobOptions(kind="album", ids=[], output_dir="d"))
        with self.assertRaises(ValueError):
            m.submit(gui_jobs.JobOptions(kind="album", ids=["a"], output_dir=""))

    def test_summary_exposes_options_and_submitted_at(self):
        o = gui_jobs.JobOptions(kind="album", ids=["1001"], output_dir="C:/d",
                                client="html", proxy="http://p", image_format="jpg",
                                image_threads=9, photo_threads=3)
        rec = gui_jobs.JobRecord(job_id="abc", options=o, submitted_at=123.0)
        s = rec.to_summary()
        self.assertEqual(s["options"]["client"], "html")
        self.assertEqual(s["options"]["imageThreads"], 9)
        self.assertEqual(s["submitted_at"], 123.0)
        # options 可直接回读为 POST /api/jobs 参数
        self.assertEqual(s["options"]["outputDir"], "C:/d")


class _InlineWorker:
    """把内联桩 worker 写入临时目录，供真实 JobManager 回环测试使用。"""

    WORKER = (
        "import sys, json, time\n"
        "def emit(ev):\n"
        "    sys.stdout.write(json.dumps(ev)+'\\n'); sys.stdout.flush()\n"
        "time.sleep(0.15)\n"
        "emit({'type':'log','line':'stub ok'})\n"
        "emit({'type':'status','state':'done','returncode':0})\n"
    )

    def __enter__(self):
        import tempfile
        self._dir = tempfile.TemporaryDirectory()
        self.path = Path(self._dir.name) / "stub_worker.py"
        self.path.write_text(self.WORKER, encoding="utf-8")
        self._saved = os.environ.get("JM_WORKER_OVERRIDE")
        os.environ["JM_WORKER_OVERRIDE"] = str(self.path)
        return self

    def __exit__(self, *exc):
        if self._saved is None:
            os.environ.pop("JM_WORKER_OVERRIDE", None)
        else:
            os.environ["JM_WORKER_OVERRIDE"] = self._saved
        self._dir.cleanup()


class JobPersistenceTest(unittest.TestCase):
    """真实 JobManager：终态落盘 + 重启回填（重启后下载视图仍展示历史终态任务）。"""

    def test_terminal_persisted_and_reseeded(self):
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            history_file = root / "jobs_history.json"
            out_dir = root / "out"
            with _InlineWorker():
                m = gui_jobs.JobManager(root=root, max_concurrent=1,
                                        history_path=str(history_file))
                rec = m.submit(gui_jobs.JobOptions(
                    kind="album", ids=["2002"], output_dir=str(out_dir)))
                # 等待终态
                for _ in range(100):
                    cur = m.get(rec.job_id)
                    if cur and cur.state in gui_jobs.TERMINAL_STATES:
                        break
                    time.sleep(0.1)
                self.assertEqual(m.get(rec.job_id).state, "done")
                self.assertTrue(history_file.exists())
                hist = json.loads(history_file.read_text(encoding="utf-8"))
                self.assertEqual(len(hist), 1)
                self.assertEqual(hist[0]["state"], "done")
                self.assertEqual(hist[0]["ids"], ["2002"])
                self.assertIn("options", hist[0])

            # 模拟重启：新 manager 读同一历史文件 → 任务回填到活动列表
            m2 = gui_jobs.JobManager(root=root, max_concurrent=1,
                                     history_path=str(history_file))
            jobs = m2.list()
            self.assertEqual(len(jobs), 1)
            self.assertEqual(jobs[0]["state"], "done")
            self.assertEqual(m2.stats()["total"], 1)

    def test_shutdown_all_cancels_active_jobs(self):
        """退出路径：shutdown_all 终止运行中的 worker → 任务收敛 cancelled。"""
        import tempfile
        sleeper = (
            "import time\n"
            "time.sleep(30)\n"
        )
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            stub = root / "sleeper_worker.py"
            stub.write_text(sleeper, encoding="utf-8")
            saved = os.environ.get("JM_WORKER_OVERRIDE")
            os.environ["JM_WORKER_OVERRIDE"] = str(stub)
            try:
                m = gui_jobs.JobManager(root=root, max_concurrent=1,
                                        history_path=str(root / "h.json"))
                rec = m.submit(gui_jobs.JobOptions(
                    kind="album", ids=["3001"], output_dir=str(root / "out")))
                for _ in range(100):
                    if m.get(rec.job_id).state == "running":
                        break
                    time.sleep(0.1)
                self.assertEqual(m.get(rec.job_id).state, "running")
                m.shutdown_all()
                for _ in range(100):
                    if m.get(rec.job_id).state in gui_jobs.TERMINAL_STATES:
                        break
                    time.sleep(0.1)
                self.assertEqual(m.get(rec.job_id).state, "cancelled")
                proc = m.get(rec.job_id).process
                self.assertIsNotNone(proc)
                self.assertIsNotNone(proc.poll())  # 子进程确已退出
            finally:
                if saved is None:
                    os.environ.pop("JM_WORKER_OVERRIDE", None)
                else:
                    os.environ["JM_WORKER_OVERRIDE"] = saved

    def test_history_limit_and_remove(self):
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            m = gui_jobs.JobManager(root=root, history_path=str(root / "h.json"),
                                    history_limit=5)
            for i in range(8):
                rec = gui_jobs.JobRecord(job_id=f"j{i}", options=gui_jobs.JobOptions(
                    kind="album", ids=[str(i)], output_dir="d"))
                rec.state = "done"
                rec.finished_at = time.time()
                m._remember_terminal(rec)
            self.assertLessEqual(len(m.history()), 5)  # 最新 5 条，最新在前
            newest = m.history()[0]
            self.assertEqual(newest["job_id"], "j7")
            self.assertTrue(m.remove_history("j7"))
            self.assertFalse(any(h["job_id"] == "j7" for h in m.history()))
            self.assertEqual(m.clear_history(), 4)


# --------------------------------------------------------------------------- #
# HTTP 路由
# --------------------------------------------------------------------------- #

class _BaseHttp(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._saved_jobs = gui_server.JOBS
        cls.manager = _StubManager()
        gui_server.JOBS = cls.manager  # type: ignore[assignment]
        cls.port = _free_port()
        cls.srv = gui_server.create_server(cls.port)
        cls.srv.start()
        cls.base = f"http://127.0.0.1:{cls.port}"

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        gui_server.JOBS = cls._saved_jobs  # type: ignore[assignment]


class JobRoutesTest(_BaseHttp):
    def test_post_jobs_validation_empty_ids(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": [], "options": {}})
        self.assertEqual(status, 400)
        data = json.loads(body)
        self.assertFalse(data["ok"])
        self.assertEqual(data["code"], "bad_request")

    def test_post_jobs_validation_bad_kind(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "wrong", "ids": ["1"], "options": {}})
        self.assertEqual(status, 400)

    def test_post_jobs_validation_non_string_id(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": [123], "options": {}})
        self.assertEqual(status, 400)

    def test_post_jobs_happy_path(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": ["1001"],
                                      "options": {"imageThreads": 8, "photoThreads": 2}})
        self.assertEqual(status, 202)
        data = json.loads(body)
        self.assertTrue(data["ok"])
        rec = data["job"]
        self.assertEqual(rec["kind"], "album")
        self.assertEqual(rec["ids"], ["1001"])
        self.assertIn(rec["job_id"], self.manager.records)

    def test_get_jobs_list(self):
        # 本用例自身先建一个
        _http("POST", f"{self.base}/api/jobs",
              body={"kind": "album", "ids": ["1234"], "options": {}})
        status, _, body = _http("GET", f"{self.base}/api/jobs")
        self.assertEqual(status, 200)
        data = json.loads(body)
        self.assertTrue(data["ok"])
        self.assertIsInstance(data["jobs"], list)
        self.assertGreaterEqual(len(data["jobs"]), 1)

    def test_get_job_detail_404(self):
        status, _, body = _http("GET", f"{self.base}/api/jobs/UNKNOWN_ID")
        self.assertEqual(status, 404)
        self.assertFalse(json.loads(body)["ok"])

    def test_get_job_detail_ok(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "photo", "ids": ["77"], "options": {}})
        job_id = json.loads(body)["job"]["job_id"]
        status, _, body = _http("GET", f"{self.base}/api/jobs/{job_id}")
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body)["job"]["job_id"], job_id)

    def test_delete_job(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": ["5"], "options": {}})
        job_id = json.loads(body)["job"]["job_id"]
        status, _, body = _http("DELETE", f"{self.base}/api/jobs/{job_id}")
        self.assertEqual(status, 200)
        self.assertTrue(json.loads(body)["ok"])
        status, _, body = _http("GET", f"{self.base}/api/jobs/{job_id}")
        self.assertEqual(json.loads(body)["job"]["state"], "cancelled")

    def test_delete_unknown(self):
        status, _, _ = _http("DELETE", f"{self.base}/api/jobs/NOPE")
        self.assertEqual(status, 404)

    def test_get_jobs_includes_stats(self):
        status, _, body = _http("GET", f"{self.base}/api/jobs")
        self.assertEqual(status, 200)
        data = json.loads(body)
        self.assertIn("stats", data)
        stats = data["stats"]
        self.assertGreaterEqual(stats["maxConcurrent"], 1)
        self.assertIn("total", stats)

    def test_delete_terminal_removes_job(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": ["5"], "options": {}})
        job_id = json.loads(body)["job"]["job_id"]
        rec = self.manager.records[job_id]
        rec.state = "done"
        rec.returncode = 0
        rec.finished_at = time.time()
        status, _, body = _http("DELETE", f"{self.base}/api/jobs/{job_id}")
        self.assertEqual(status, 200)
        self.assertTrue(json.loads(body).get("removed"))
        self.assertNotIn(job_id, self.manager.records)

    def test_delete_active_still_cancels(self):
        # 活跃任务 DELETE 保持“取消”语义
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "album", "ids": ["5"], "options": {}})
        job_id = json.loads(body)["job"]["job_id"]
        status, _, body = _http("DELETE", f"{self.base}/api/jobs/{job_id}")
        self.assertEqual(status, 200)
        self.assertTrue(json.loads(body).get("cancelled"))

    def test_history_empty_and_clear(self):
        status, _, body = _http("GET", f"{self.base}/api/history")
        self.assertEqual(status, 200)
        self.assertIsInstance(json.loads(body)["history"], list)
        status, _, body = _http("DELETE", f"{self.base}/api/history")
        self.assertEqual(status, 200)

    def test_history_record_and_remove(self):
        status, _, body = _http("POST", f"{self.base}/api/jobs",
                                body={"kind": "photo", "ids": ["7"], "options": {}})
        job_id = json.loads(body)["job"]["job_id"]
        # 模拟历史记录已写入（真实 manager 在终态写入）
        rec = self.manager.records[job_id]
        self.manager._history.append(rec.to_summary())
        status, _, body = _http("GET", f"{self.base}/api/history")
        self.assertEqual(status, 200)
        items = json.loads(body)["history"]
        self.assertTrue(any(h["job_id"] == job_id for h in items))
        # 单条删除
        status, _, body = _http("DELETE", f"{self.base}/api/history/{job_id}")
        self.assertEqual(status, 200)
        status, _, body = _http("GET", f"{self.base}/api/history")
        items = json.loads(body)["history"]
        self.assertFalse(any(h["job_id"] == job_id for h in items))


# --------------------------------------------------------------------------- #
# SSE
# --------------------------------------------------------------------------- #

class SseStreamTest(_BaseHttp):
    def test_sse_replay_then_terminal(self):
        rec = self.manager.submit(gui_jobs.JobOptions(kind="album", ids=["2002"],
                                                     output_dir="d:/x"))
        rec.state = "running"
        rec.started_at = time.time()
        rec.events.append({"type": "status", "state": "running", "pid": 9999})
        rec.events.append({"type": "log", "line": "启动中..."})
        rec.events.append({"type": "log", "line": "完成"})
        rec.state = "done"
        rec.returncode = 0
        rec.summary = {"ok": True}
        rec.events.append({"type": "status", "state": "done", "returncode": 0})

        req = urlrequest.Request(f"{self.base}/api/jobs/{rec.job_id}/events", method="GET")
        try:
            resp = urlrequest.urlopen(req, timeout=4)
        except Exception as exc:
            self.fail("SSE 连接失败: " + str(exc))
        try:
            ct = resp.headers.get("Content-Type", "")
        finally:
            data = resp.read()
        self.assertIn("text/event-stream", ct)
        self.assertIn(b"event: status", data)
        self.assertIn(b"event: log", data)
        self.assertIn(b'"state": "running"', data)
        self.assertIn(b'"state": "done"', data)
        self.assertIn(b"\xe5\xae\x8c\xe6\x88\x90", data)  # “完成” UTF-8


if __name__ == "__main__":
    unittest.main()
