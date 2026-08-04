from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import traceback
import unicodedata
from pathlib import Path
import tkinter as tk
from tkinter import filedialog, messagebox, ttk


APP_NAME = "JMComic-Downloader-GUI"
APP_VERSION = "1.2.0"
BG = "#f3f5f7"
PANEL = "#ffffff"
TEXT = "#1f2933"
MUTED = "#64707d"
ACCENT = "#16856b"
ACCENT_DARK = "#116c58"
DANGER = "#c43d4d"

PROXY_SYSTEM = "跟随系统"
PROXY_NONE = "不使用代理"
FORMAT_LABELS = {
    "保持原格式": "original",
    "统一为 JPG": "jpg",
    "统一为 PNG": "png",
    "统一为 WebP": "webp",
}
CLIENT_LABELS = {
    "移动端 API（推荐）": "api",
    "网页端 HTML": "html",
}


def app_directory() -> Path:
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent


def default_download_directory() -> Path:
    return app_directory() / "下载"


def paths_equal(left: str | Path, right: str | Path) -> bool:
    return os.path.normcase(os.path.abspath(os.fspath(left))) == os.path.normcase(
        os.path.abspath(os.fspath(right))
    )


def initial_output_directory(settings: dict) -> Path:
    default = default_download_directory()
    saved = str(settings.get("output", "") or "").strip()
    output_mode = settings.get("output_mode")

    if output_mode == "custom" and saved:
        return Path(saved)
    if output_mode == "default":
        return default

    # 兼容旧设置：旧版会把程序目录下的“下载”绝对路径写入设置。
    # 程序被移动或重新解压后，应让这种旧默认值跟随新的 EXE 目录。
    if saved and Path(saved).name != "下载":
        return Path(saved)
    return default


def settings_path() -> Path:
    local_app_data = os.environ.get("LOCALAPPDATA")
    base = Path(local_app_data) if local_app_data else Path.home() / "AppData" / "Local"
    return base / "JMComic-Downloader-GUI" / "settings.json"


def legacy_settings_path() -> Path:
    local_app_data = os.environ.get("LOCALAPPDATA")
    base = Path(local_app_data) if local_app_data else Path.home() / "AppData" / "Local"
    return base / "JMComicDownloader" / "settings.json"


def parse_ids(raw_text: str) -> list[str]:
    tokens = [item for item in re.split(r"[\s,;，；]+", raw_text.strip()) if item]
    if not tokens:
        raise ValueError("请至少输入一个 JM 车号或链接")

    patterns = (
        re.compile(r"/(?:album|photo)/(?:\?id=)?(\d+)", re.IGNORECASE),
        re.compile(r"[?&]id=(\d+)", re.IGNORECASE),
    )
    result: list[str] = []
    seen: set[str] = set()

    for token in tokens:
        value = token
        if value[:2].lower() == "jm":
            value = value[2:]
        elif value[:1].lower() in {"a", "p"} and value[1:].isdigit():
            value = value[1:]

        if not value.isdigit():
            matched = next((pattern.search(token) for pattern in patterns if pattern.search(token)), None)
            if matched is not None:
                value = matched.group(1)

        if not value.isdigit():
            raise ValueError(f"无法识别：{token}")

        value = value.lstrip("0") or "0"
        if value not in seen:
            result.append(value)
            seen.add(value)

    return result


def normalize_proxy(proxy: str):
    from common import ProxyBuilder

    proxy = proxy.strip()
    if proxy == "system":
        return ProxyBuilder.system_proxy()
    if proxy == "none" or not proxy:
        return {}
    return ProxyBuilder.build_by_str(proxy)


def safe_pdf_stem(value: str, fallback: str, max_length: int = 180) -> str:
    value = unicodedata.normalize("NFKC", str(value or ""))
    value = re.sub(r"[\\/:*?\"<>|\x00-\x1f]", "_", value)
    value = re.sub(r"\s+", " ", value).strip(" .")
    if not value:
        value = fallback

    reserved = {
        "CON", "PRN", "AUX", "NUL",
        *(f"COM{i}" for i in range(1, 10)),
        *(f"LPT{i}" for i in range(1, 10)),
    }
    if value.upper() in reserved:
        value = f"_{value}"

    # 为完整路径和章节后缀留出余量，避免 Windows 路径过长。
    value = value[:max_length].rstrip(" .")
    return value or fallback


def pdf_filename_for_photo(album, photo) -> str:
    album_id = str(getattr(album, "id", getattr(album, "album_id", "unknown")))
    album_name = safe_pdf_stem(getattr(album, "name", ""), f"JM{album_id}", max_length=110)

    if len(album) <= 1:
        return f"{album_name}.pdf"

    chapter_index = getattr(photo, "index", getattr(photo, "album_index", 1))
    chapter_name = str(getattr(photo, "name", "") or "").strip()
    suffix = f"第{chapter_index}话"
    if chapter_name and chapter_name != str(getattr(album, "name", "")).strip():
        suffix += f" {chapter_name}"

    stem = safe_pdf_stem(f"{album_name} - {suffix}", f"{album_name} - 第{chapter_index}话")
    return f"{stem}.pdf"


def _result_items(result) -> list:
    if result is None:
        return []
    if hasattr(result, "detail") and hasattr(result, "downloader"):
        return [result]
    return sorted(
        (item for item in result if hasattr(item, "detail") and hasattr(item, "downloader")),
        key=lambda item: str(getattr(item.detail, "id", "")),
    )


def image_paths_for_photo(photo, option) -> list[Path]:
    image_paths: list[Path] = []
    missing: list[str] = []

    for image in photo:
        saved_path = getattr(image, "save_path", None)
        path = Path(saved_path or option.decide_image_filepath(image)).resolve()
        if path.is_file():
            image_paths.append(path)
        else:
            missing.append(str(path))

    if missing:
        preview = "\n".join(f"  - {item}" for item in missing[:3])
        remainder = f"\n  ...另有 {len(missing) - 3} 个" if len(missing) > 3 else ""
        raise FileNotFoundError(f"章节图片不完整，缺少 {len(missing)} 个文件：\n{preview}{remainder}")
    if not image_paths:
        raise ValueError("章节没有可用于生成 PDF 的图片")
    return image_paths


def write_verified_pdf(image_paths: list[Path], target_path: Path, title: str) -> int:
    import img2pdf
    from pypdf import PdfReader

    target_path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = target_path.with_name(f".{target_path.name}.{os.getpid()}.tmp")
    try:
        with temp_path.open("wb") as output:
            img2pdf.convert(
                image_paths,
                outputstream=output,
                engine=img2pdf.Engine.internal,
                rotation=img2pdf.Rotation.ifvalid,
                first_frame_only=True,
                nodate=True,
                title=title,
                creator=APP_NAME,
                producer=f"img2pdf {img2pdf.__version__}",
            )

        reader = PdfReader(str(temp_path), strict=False)
        page_count = len(reader.pages)
        if page_count != len(image_paths):
            raise ValueError(f"PDF 页数校验失败：应有 {len(image_paths)} 页，实际 {page_count} 页")

        os.replace(temp_path, target_path)
        return page_count
    finally:
        try:
            temp_path.unlink(missing_ok=True)
        except OSError:
            pass


def remove_exported_images(image_paths: list[Path], output_dir: str) -> int:
    """Delete source images only after their PDF has been verified."""
    output_root = Path(output_dir).resolve()
    pdf_root = (output_root / "PDF").resolve()
    deleted = 0
    parent_directories: set[Path] = set()

    for image_path in image_paths:
        path = image_path.resolve()
        try:
            path.relative_to(output_root)
        except ValueError as error:
            raise ValueError(f"拒绝删除下载目录外的图片：{path}") from error
        try:
            path.relative_to(pdf_root)
        except ValueError:
            pass
        else:
            raise ValueError(f"拒绝删除 PDF 目录内的文件：{path}")

        if path.is_file():
            path.unlink()
            deleted += 1
        parent_directories.add(path.parent)

    for directory in sorted(
        parent_directories,
        key=lambda item: len(item.parts),
        reverse=True,
    ):
        current = directory
        while current != output_root:
            try:
                current.rmdir()
            except OSError:
                break
            current = current.parent

    return deleted


def export_result_pdfs(result, option, output_dir: str) -> tuple[list[Path], list[tuple[str, Exception]]]:
    pdf_dir = Path(output_dir).resolve() / "PDF"
    pdf_dir.mkdir(parents=True, exist_ok=True)
    exported: list[Path] = []
    failures: list[tuple[str, Exception]] = []

    for item in _result_items(result):
        detail = item.detail
        if detail.is_album():
            album = detail
            photos = sorted(list(album), key=lambda photo: int(getattr(photo, "index", 0)))
        else:
            photos = [detail]
            album = detail.from_album

        for photo in photos:
            pdf_name = pdf_filename_for_photo(album, photo)
            target_path = pdf_dir / pdf_name
            try:
                image_paths = image_paths_for_photo(photo, option)
                page_count = write_verified_pdf(image_paths, target_path, target_path.stem)
                deleted_count = remove_exported_images(image_paths, output_dir)
                exported.append(target_path)
                print(
                    f"PDF 生成完成：{target_path}（{page_count} 页，"
                    f"已清理 {deleted_count} 张中间图片）",
                    flush=True,
                )
            except Exception as error:
                failures.append((pdf_name, error))
                print(f"PDF 生成失败：{pdf_name}，原因：{error}", flush=True)
                traceback.print_exc(file=sys.stderr)

    return exported, failures


def run_worker(args: argparse.Namespace) -> int:
    log_handle = None
    original_stdout = sys.stdout
    original_stderr = sys.stderr
    try:
        log_handle = open(args.log_file, "a", encoding="utf-8", buffering=1)
        sys.stdout = log_handle
        sys.stderr = log_handle

        print(f"[{time.strftime('%H:%M:%S')}] 正在启动 JMComic 下载核心...", flush=True)
        import jmcomic

        output_dir = os.path.abspath(args.output)
        os.makedirs(output_dir, exist_ok=True)

        if args.option:
            option = jmcomic.create_option_by_file(os.path.abspath(args.option))
            print(f"已载入高级配置：{args.option}")
        else:
            option = jmcomic.JmOption.construct(
                {"dir_rule": {"base_dir": output_dir, "rule": "Bd_Atitle_Pindex"}}
            )

        option.dir_rule.base_dir = output_dir
        option.client.impl = args.client
        option.client.postman.meta_data.proxies = normalize_proxy(args.proxy)
        option.download.image.suffix = None if args.image_format == "original" else f".{args.image_format}"
        option.download.threading.image = args.image_threads
        option.download.threading.photo = args.photo_threads

        label = "本子" if args.kind == "album" else "章节"
        print(f"下载类型：{label}")
        print(f"任务 ID：{', '.join(args.ids)}")
        print(f"保存位置：{output_dir}")
        print(f"PDF 目录：{os.path.join(output_dir, 'PDF')}")
        print("PDF 使用无损图像编码；校验成功后自动删除中间图片。")
        print("-" * 64, flush=True)

        download = jmcomic.download_album if args.kind == "album" else jmcomic.download_photo
        target = args.ids[0] if len(args.ids) == 1 else args.ids
        result = download(target, option, check_exception=True)
        failed = getattr(result, "failed", {})

        print("-" * 64)
        print("开始按章节整合 PDF...", flush=True)
        exported, pdf_failures = export_result_pdfs(result, option, output_dir)

        if failed:
            print("-" * 64)
            print(f"任务结束，但有 {len(failed)} 项失败：")
            for item_id, error in failed.items():
                print(f"  {item_id}: {error}")

        if pdf_failures:
            print("-" * 64)
            print(f"有 {len(pdf_failures)} 个 PDF 生成失败：")
            for pdf_name, error in pdf_failures:
                print(f"  {pdf_name}: {error}")

        if failed or pdf_failures:
            return 2

        print("-" * 64)
        print(f"全部完成，共处理 {len(args.ids)} 项，生成 {len(exported)} 个 PDF。", flush=True)
        return 0
    except Exception:
        traceback.print_exc(file=sys.stderr)
        return 1
    finally:
        if log_handle is not None:
            log_handle.flush()
            sys.stdout = original_stdout
            sys.stderr = original_stderr
            log_handle.close()


def run_self_test(log_file: str) -> int:
    lines: list[str] = []
    try:
        import bz2
        import ctypes
        import lzma
        import xml.parsers.expat

        import curl_cffi
        import img2pdf
        import jmcomic
        import yaml
        from Crypto.Cipher import AES
        from PIL import Image
        from pypdf import PdfReader
        from io import BytesIO

        root = tk.Tk()
        root.withdraw()
        root.update_idletasks()
        root.destroy()

        assert bz2.compress(b"jmcomic")
        assert lzma.compress(b"jmcomic")
        assert ctypes.sizeof(ctypes.c_void_p) in (4, 8)
        assert xml.parsers.expat.ParserCreate()
        assert curl_cffi.__version__
        assert jmcomic.__version__
        assert yaml.safe_load("ready: true")["ready"] is True
        assert AES.block_size == 16
        assert Image.new("RGB", (1, 1)).size == (1, 1)
        image_buffer = BytesIO()
        Image.new("RGB", (200, 300), "white").save(image_buffer, format="PNG")
        pdf_data = img2pdf.convert(
            image_buffer.getvalue(),
            engine=img2pdf.Engine.internal,
            nodate=True,
            first_frame_only=True,
        )
        assert len(PdfReader(BytesIO(pdf_data)).pages) == 1
        lines.append(f"self-test: OK (jmcomic {jmcomic.__version__})")
        code = 0
    except Exception:
        lines.append("self-test: FAILED")
        lines.append(traceback.format_exc())
        code = 1

    if log_file:
        try:
            Path(log_file).write_text("\n".join(lines), encoding="utf-8")
        except OSError:
            return 2
    return code


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--kind", choices=("album", "photo"), default="album")
    parser.add_argument("--id", dest="ids", action="append", default=[])
    parser.add_argument("--output", default="")
    parser.add_argument("--client", choices=("api", "html"), default="api")
    parser.add_argument("--image-format", choices=("original", "jpg", "png", "webp"), default="original")
    parser.add_argument("--image-threads", type=int, default=20)
    parser.add_argument("--photo-threads", type=int, default=4)
    parser.add_argument("--proxy", default="system")
    parser.add_argument("--option", default="")
    parser.add_argument("--log-file", default="")
    return parser


class JMComicApp:
    def __init__(self, root: tk.Tk):
        self.root = root
        self.process: subprocess.Popen | None = None
        self.log_file: Path | None = None
        self.log_offset = 0
        self.stop_requested = False
        self.busy_widgets: list[tk.Widget] = []
        self.settings = self._load_settings()

        self.mode_var = tk.StringVar(value=self.settings.get("mode", "album"))
        self.output_var = tk.StringVar(value=str(initial_output_directory(self.settings)))
        self.client_var = tk.StringVar(value=self.settings.get("client", "移动端 API（推荐）"))
        self.format_var = tk.StringVar(value=self.settings.get("format", "保持原格式"))
        self.image_threads_var = tk.IntVar(value=self.settings.get("image_threads", 20))
        self.photo_threads_var = tk.IntVar(value=self.settings.get("photo_threads", 4))
        self.proxy_var = tk.StringVar(value=self.settings.get("proxy", PROXY_SYSTEM))
        self.option_var = tk.StringVar(value=self.settings.get("option", ""))
        self.status_var = tk.StringVar(value="就绪")

        self._configure_window()
        self._configure_styles()
        self._build_ui()
        self.root.protocol("WM_DELETE_WINDOW", self._on_close)

    def _configure_window(self):
        self.root.title(f"{APP_NAME} {APP_VERSION}")
        self.root.geometry("1000x720")
        self.root.minsize(900, 700)
        self.root.configure(bg=BG)
        self.root.option_add("*Font", ("Microsoft YaHei UI", 10))

    def _configure_styles(self):
        style = ttk.Style(self.root)
        try:
            style.theme_use("vista")
        except tk.TclError:
            pass
        style.configure("App.TFrame", background=BG)
        style.configure("Panel.TFrame", background=PANEL)
        style.configure("Header.TFrame", background="#202830")
        style.configure("HeaderTitle.TLabel", background="#202830", foreground="#ffffff", font=("Microsoft YaHei UI", 16, "bold"))
        style.configure("HeaderSub.TLabel", background="#202830", foreground="#b9c4cd", font=("Microsoft YaHei UI", 9))
        style.configure("Section.TLabel", background=PANEL, foreground=TEXT, font=("Microsoft YaHei UI", 10, "bold"))
        style.configure("Hint.TLabel", background=PANEL, foreground=MUTED, font=("Microsoft YaHei UI", 9))
        style.configure("Status.TLabel", background=BG, foreground=MUTED)
        style.configure("Accent.TButton", foreground="#ffffff", background=ACCENT, padding=(18, 8), font=("Microsoft YaHei UI", 10, "bold"))
        style.map("Accent.TButton", background=[("active", ACCENT_DARK), ("disabled", "#aab5b1")])
        style.configure("Danger.TButton", foreground=DANGER, padding=(12, 7))
        style.configure("TButton", padding=(10, 6))
        style.configure("TEntry", padding=5)
        style.configure("TCombobox", padding=4)
        style.configure("Horizontal.TProgressbar", troughcolor="#dfe5e8", background=ACCENT)

    def _build_ui(self):
        header = ttk.Frame(self.root, style="Header.TFrame", padding=(26, 12))
        header.pack(fill="x")
        ttk.Label(header, text="JMComic Downloader GUI", style="HeaderTitle.TLabel").pack(anchor="w")
        ttk.Label(header, text="批量下载 · 无损整合 PDF · 自动清理图片 · 实时日志", style="HeaderSub.TLabel").pack(anchor="w", pady=(3, 0))

        content = ttk.Frame(self.root, style="App.TFrame", padding=(20, 12, 20, 8))
        content.pack(fill="both", expand=True)
        content.columnconfigure(0, weight=0, minsize=330)
        content.columnconfigure(1, weight=1)
        content.rowconfigure(0, weight=1)

        controls = ttk.Frame(content, style="Panel.TFrame", padding=(18, 10))
        controls.grid(row=0, column=0, sticky="nsew", padx=(0, 14))
        controls.columnconfigure(0, weight=1)

        ttk.Label(controls, text="下载类型", style="Section.TLabel").grid(row=0, column=0, sticky="w")
        mode_frame = tk.Frame(controls, bg=PANEL)
        mode_frame.grid(row=1, column=0, sticky="ew", pady=(5, 10))
        mode_frame.columnconfigure((0, 1), weight=1, uniform="mode")
        self.album_button = tk.Radiobutton(
            mode_frame, text="本子", variable=self.mode_var, value="album", indicatoron=False,
            bg="#e8ecef", selectcolor="#bfe4d9", activebackground="#d8e4e1", fg=TEXT,
            relief="flat", bd=0, padx=12, pady=7,
        )
        self.album_button.grid(row=0, column=0, sticky="ew", padx=(0, 3))
        self.photo_button = tk.Radiobutton(
            mode_frame, text="章节", variable=self.mode_var, value="photo", indicatoron=False,
            bg="#e8ecef", selectcolor="#bfe4d9", activebackground="#d8e4e1", fg=TEXT,
            relief="flat", bd=0, padx=12, pady=7,
        )
        self.photo_button.grid(row=0, column=1, sticky="ew", padx=(3, 0))
        self.busy_widgets.extend([self.album_button, self.photo_button])

        ttk.Label(controls, text="车号或链接", style="Section.TLabel").grid(row=2, column=0, sticky="w")
        ttk.Label(controls, text="支持多个，以空格、逗号或换行分隔", style="Hint.TLabel").grid(row=3, column=0, sticky="w", pady=(2, 6))
        self.id_text = tk.Text(
            controls, width=38, height=4, wrap="word", relief="solid", bd=1,
            highlightthickness=0, bg="#fbfcfd", fg=TEXT, insertbackground=TEXT,
            padx=9, pady=8, font=("Consolas", 10),
        )
        self.id_text.grid(row=4, column=0, sticky="ew", pady=(0, 10))
        self.busy_widgets.append(self.id_text)

        ttk.Label(controls, text="保存位置", style="Section.TLabel").grid(row=5, column=0, sticky="w")
        output_row = ttk.Frame(controls, style="Panel.TFrame")
        output_row.grid(row=6, column=0, sticky="ew", pady=(5, 10))
        output_row.columnconfigure(0, weight=1)
        self.output_entry = ttk.Entry(output_row, textvariable=self.output_var)
        self.output_entry.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        output_browse = ttk.Button(output_row, text="选择...", command=self._choose_output)
        output_browse.grid(row=0, column=1)
        self.busy_widgets.extend([self.output_entry, output_browse])

        settings_grid = ttk.Frame(controls, style="Panel.TFrame")
        settings_grid.grid(row=7, column=0, sticky="ew")
        settings_grid.columnconfigure((0, 1), weight=1, uniform="settings")

        ttk.Label(settings_grid, text="客户端", style="Hint.TLabel").grid(row=0, column=0, sticky="w")
        ttk.Label(settings_grid, text="图片格式", style="Hint.TLabel").grid(row=0, column=1, sticky="w", padx=(8, 0))
        self.client_combo = ttk.Combobox(settings_grid, textvariable=self.client_var, values=list(CLIENT_LABELS), state="readonly")
        self.client_combo.grid(row=1, column=0, sticky="ew", pady=(3, 7))
        self.format_combo = ttk.Combobox(settings_grid, textvariable=self.format_var, values=list(FORMAT_LABELS), state="readonly")
        self.format_combo.grid(row=1, column=1, sticky="ew", padx=(8, 0), pady=(3, 7))

        ttk.Label(settings_grid, text="图片并发", style="Hint.TLabel").grid(row=2, column=0, sticky="w")
        ttk.Label(settings_grid, text="章节并发", style="Hint.TLabel").grid(row=2, column=1, sticky="w", padx=(8, 0))
        self.image_spin = ttk.Spinbox(settings_grid, from_=1, to=50, textvariable=self.image_threads_var, width=8)
        self.image_spin.grid(row=3, column=0, sticky="ew", pady=(3, 7))
        self.photo_spin = ttk.Spinbox(settings_grid, from_=1, to=32, textvariable=self.photo_threads_var, width=8)
        self.photo_spin.grid(row=3, column=1, sticky="ew", padx=(8, 0), pady=(3, 7))
        self.busy_widgets.extend([self.client_combo, self.format_combo, self.image_spin, self.photo_spin])

        ttk.Label(controls, text="代理", style="Hint.TLabel").grid(row=8, column=0, sticky="w")
        self.proxy_combo = ttk.Combobox(
            controls, textvariable=self.proxy_var,
            values=(PROXY_SYSTEM, PROXY_NONE, "127.0.0.1:7890", "127.0.0.1:10809"),
        )
        self.proxy_combo.grid(row=9, column=0, sticky="ew", pady=(3, 8))
        self.busy_widgets.append(self.proxy_combo)

        ttk.Label(controls, text="高级 YAML 配置（可选）", style="Hint.TLabel").grid(row=10, column=0, sticky="w")
        option_row = ttk.Frame(controls, style="Panel.TFrame")
        option_row.grid(row=11, column=0, sticky="ew", pady=(3, 0))
        option_row.columnconfigure(0, weight=1)
        self.option_entry = ttk.Entry(option_row, textvariable=self.option_var)
        self.option_entry.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        option_browse = ttk.Button(option_row, text="选择...", command=self._choose_option)
        option_browse.grid(row=0, column=1)
        option_clear = ttk.Button(option_row, text="清除", command=lambda: self.option_var.set(""))
        option_clear.grid(row=0, column=2, padx=(5, 0))
        self.busy_widgets.extend([self.option_entry, option_browse, option_clear])

        log_panel = ttk.Frame(content, style="Panel.TFrame", padding=(18, 10))
        log_panel.grid(row=0, column=1, sticky="nsew")
        log_panel.columnconfigure(0, weight=1)
        log_panel.rowconfigure(2, weight=1)
        ttk.Label(log_panel, text="运行日志", style="Section.TLabel").grid(row=0, column=0, sticky="w")
        ttk.Label(log_panel, text="下载进度、重试和错误会显示在这里", style="Hint.TLabel").grid(row=1, column=0, sticky="w", pady=(2, 6))
        log_frame = tk.Frame(log_panel, bg=PANEL)
        log_frame.grid(row=2, column=0, sticky="nsew")
        log_frame.columnconfigure(0, weight=1)
        log_frame.rowconfigure(0, weight=1)
        self.log_text = tk.Text(
            log_frame, width=54, state="disabled", wrap="word", bg="#182027", fg="#d7e2e7",
            insertbackground="#ffffff", selectbackground="#34535f", relief="flat",
            padx=12, pady=10, font=("Consolas", 9),
        )
        self.log_text.grid(row=0, column=0, sticky="nsew")
        log_scroll = ttk.Scrollbar(log_frame, orient="vertical", command=self.log_text.yview)
        log_scroll.grid(row=0, column=1, sticky="ns")
        self.log_text.configure(yscrollcommand=log_scroll.set)
        self._append_log("等待任务。输入车号后点击“开始下载”。\n")

        footer = ttk.Frame(self.root, style="App.TFrame", padding=(20, 0, 20, 12))
        footer.pack(fill="x")
        footer.columnconfigure(0, minsize=150)
        footer.columnconfigure(1, weight=1)
        self.progress = ttk.Progressbar(footer, mode="indeterminate", length=150)
        self.progress.grid(row=0, column=0, sticky="w")
        self.progress.grid_remove()
        ttk.Label(footer, textvariable=self.status_var, style="Status.TLabel").grid(row=0, column=1, sticky="w", padx=(10, 0))
        self.open_button = ttk.Button(footer, text="打开 PDF 目录", command=self._open_output)
        self.open_button.grid(row=0, column=2, padx=(8, 0))
        self.stop_button = ttk.Button(footer, text="停止", command=self._stop_download, style="Danger.TButton", state="disabled")
        self.stop_button.grid(row=0, column=3, padx=(8, 0))
        self.start_button = tk.Button(
            footer, text="开始下载", command=self._start_download,
            bg=ACCENT, activebackground=ACCENT_DARK, fg="#ffffff", activeforeground="#ffffff",
            disabledforeground="#edf1ef", relief="flat", bd=0, padx=18, pady=7,
            font=("Microsoft YaHei UI", 10, "bold"), cursor="hand2",
        )
        self.start_button.grid(row=0, column=4, padx=(8, 0))

    def _load_settings(self) -> dict:
        for path in (settings_path(), legacy_settings_path()):
            try:
                return json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError, TypeError):
                continue
        return {}

    def _save_settings(self):
        output = self.output_var.get().strip()
        uses_default_output = not output or paths_equal(output, default_download_directory())
        data = {
            "mode": self.mode_var.get(),
            "output_mode": "default" if uses_default_output else "custom",
            "output": "" if uses_default_output else output,
            "client": self.client_var.get(),
            "format": self.format_var.get(),
            "image_threads": self.image_threads_var.get(),
            "photo_threads": self.photo_threads_var.get(),
            "proxy": self.proxy_var.get().strip(),
            "option": self.option_var.get().strip(),
        }
        path = settings_path()
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
        except OSError:
            pass

    def _choose_output(self):
        initial = self.output_var.get().strip() or str(app_directory())
        selected = filedialog.askdirectory(title="选择下载目录", initialdir=initial)
        if selected:
            self.output_var.set(selected)

    def _choose_option(self):
        selected = filedialog.askopenfilename(
            title="选择 JMComic 配置文件",
            filetypes=(("YAML 配置", "*.yml *.yaml"), ("所有文件", "*.*")),
            initialdir=str(app_directory()),
        )
        if selected:
            self.option_var.set(selected)

    def _open_output(self):
        path = Path(self.output_var.get().strip() or default_download_directory()) / "PDF"
        try:
            path.mkdir(parents=True, exist_ok=True)
            os.startfile(str(path))
        except OSError as error:
            messagebox.showerror(APP_NAME, f"无法打开目录：\n{error}")

    def _proxy_arg(self) -> str:
        value = self.proxy_var.get().strip()
        if value == PROXY_SYSTEM:
            return "system"
        if value == PROXY_NONE:
            return "none"
        return value

    def _validated_job(self) -> dict:
        ids = parse_ids(self.id_text.get("1.0", "end"))
        output = self.output_var.get().strip()
        if not output:
            raise ValueError("请选择下载保存位置")

        image_threads = int(self.image_threads_var.get())
        photo_threads = int(self.photo_threads_var.get())
        if not 1 <= image_threads <= 50:
            raise ValueError("图片并发必须在 1 到 50 之间")
        if not 1 <= photo_threads <= 32:
            raise ValueError("章节并发必须在 1 到 32 之间")

        option = self.option_var.get().strip()
        if option and not Path(option).is_file():
            raise ValueError("高级 YAML 配置文件不存在")

        return {
            "ids": ids,
            "output": os.path.abspath(output),
            "image_threads": image_threads,
            "photo_threads": photo_threads,
            "option": option,
        }

    def _worker_command(self, job: dict) -> list[str]:
        if getattr(sys, "frozen", False):
            command = [sys.executable]
        else:
            command = [sys.executable, str(Path(__file__).resolve())]

        command.extend(
            [
                "--worker",
                "--kind", self.mode_var.get(),
                "--output", job["output"],
                "--client", CLIENT_LABELS[self.client_var.get()],
                "--image-format", FORMAT_LABELS[self.format_var.get()],
                "--image-threads", str(job["image_threads"]),
                "--photo-threads", str(job["photo_threads"]),
                "--proxy", self._proxy_arg(),
                "--log-file", str(self.log_file),
            ]
        )
        if job["option"]:
            command.extend(["--option", job["option"]])
        for item_id in job["ids"]:
            command.extend(["--id", item_id])
        return command

    def _start_download(self):
        if self.process is not None:
            return
        try:
            job = self._validated_job()
            Path(job["output"]).mkdir(parents=True, exist_ok=True)
        except (ValueError, OSError) as error:
            messagebox.showwarning(APP_NAME, str(error))
            return

        fd, log_name = tempfile.mkstemp(prefix="jmcomic_", suffix=".log")
        os.close(fd)
        self.log_file = Path(log_name)
        self.log_offset = 0
        self.stop_requested = False
        self._clear_log()
        self._append_log(f"已提交 {len(job['ids'])} 项下载任务。\n")
        self._save_settings()

        try:
            startupinfo = subprocess.STARTUPINFO()
            startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            self.process = subprocess.Popen(
                self._worker_command(job),
                cwd=job["output"],
                startupinfo=startupinfo,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
        except OSError as error:
            self._cleanup_log_file()
            self.process = None
            messagebox.showerror(APP_NAME, f"无法启动下载任务：\n{error}")
            return

        self._set_busy(True)
        self.status_var.set("正在下载")
        self.progress.grid()
        self.progress.start(12)
        self.root.after(120, self._poll_process)

    def _poll_process(self):
        if self.process is None:
            return
        self._read_new_log()
        return_code = self.process.poll()
        if return_code is None:
            self.root.after(180, self._poll_process)
            return

        self._read_new_log()
        self.progress.stop()
        self.progress.grid_remove()
        self._set_busy(False)
        self.process = None

        if self.stop_requested:
            self.status_var.set("任务已停止")
            self._append_log("\n任务已由用户停止。\n")
        elif return_code == 0:
            self.status_var.set("下载及 PDF 整合完成")
            self.root.bell()
        else:
            self.status_var.set("下载失败，请查看日志")
            self.root.bell()

        self._cleanup_log_file()

    def _read_new_log(self):
        if self.log_file is None:
            return
        try:
            with self.log_file.open("r", encoding="utf-8", errors="replace") as handle:
                handle.seek(self.log_offset)
                chunk = handle.read()
                self.log_offset = handle.tell()
            if chunk:
                self._append_log(chunk)
        except OSError:
            pass

    def _stop_download(self):
        if self.process is None or self.process.poll() is not None:
            return
        self.stop_requested = True
        self.status_var.set("正在停止...")
        self._terminate_process_tree()

    def _terminate_process_tree(self):
        if self.process is None or self.process.poll() is not None:
            return
        try:
            startupinfo = subprocess.STARTUPINFO()
            startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            result = subprocess.run(
                ["taskkill", "/PID", str(self.process.pid), "/T", "/F"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                startupinfo=startupinfo,
                creationflags=subprocess.CREATE_NO_WINDOW,
                timeout=8,
                check=False,
            )
            if result.returncode == 0:
                return
        except (OSError, subprocess.TimeoutExpired):
            pass
        try:
            self.process.terminate()
        except OSError:
            pass

    def _set_busy(self, busy: bool):
        for widget in self.busy_widgets:
            try:
                if isinstance(widget, ttk.Combobox):
                    widget.configure(state="disabled" if busy else ("readonly" if widget in (self.client_combo, self.format_combo) else "normal"))
                else:
                    widget.configure(state="disabled" if busy else "normal")
            except tk.TclError:
                pass
        self.start_button.configure(
            state="disabled" if busy else "normal",
            bg="#8fa39d" if busy else ACCENT,
        )
        self.stop_button.configure(state="normal" if busy else "disabled")

    def _append_log(self, text: str):
        self.log_text.configure(state="normal")
        self.log_text.insert("end", text)
        self.log_text.see("end")
        self.log_text.configure(state="disabled")

    def _clear_log(self):
        self.log_text.configure(state="normal")
        self.log_text.delete("1.0", "end")
        self.log_text.configure(state="disabled")

    def _cleanup_log_file(self):
        if self.log_file is not None:
            try:
                self.log_file.unlink(missing_ok=True)
            except OSError:
                pass
        self.log_file = None

    def _on_close(self):
        if self.process is not None and self.process.poll() is None:
            if not messagebox.askyesno(APP_NAME, "下载仍在进行，确定停止任务并退出吗？"):
                return
            self._terminate_process_tree()
        self._save_settings()
        self._cleanup_log_file()
        self.root.destroy()


def main() -> int:
    parser = build_parser()
    args, _unknown = parser.parse_known_args()
    if args.self_test:
        return run_self_test(args.log_file)
    if args.worker:
        if not args.log_file or not args.output or not args.ids:
            return 64
        return run_worker(args)

    root = tk.Tk()
    JMComicApp(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
