# JMComic-Downloader-GUI

Windows 图形化批量下载器：支持车号/链接输入，多线程同步下载图片，几乎无损的 PDF 整合，以及附带了 Chromium 浏览器标签页批量收集插件。

> 本项目基于对开源项目 `hect0x7/JMComic-Crawler-Python` 进行的二次开发，保留上游项目的 MIT License 及原作者 hect0x7 的版权声明，并在此基础上新增了 Windows GUI、多线程下载、PDF 整合和浏览器扩展等功能。不属于 JMComic、18comic 或上游作者的官方发行版。上游来源与修改说明见 [UPSTREAM.md](UPSTREAM.md)。

## 下载

请前往 GitHub 仓库的 **Releases** 页面，下载：

```text
JMComic-Downloader-GUI-1.2.0-Windows-x64.zip
```

下载后完整解压，再运行：

```text
JMComic-Downloader-GUI.exe
```

使用说明见解压后的 “使用说明.md”

附：Release 同时提供 `.sha256.txt` 校验文件。Windows SmartScreen 或杀毒软件可能对未签名的 PyInstaller 程序显示提示；请从正式 Release 下载并核对 SHA256。

## 主要功能

- Windows 10/11 图形界面，无需命令行。
- 支持单个或批量输入 JM 车号、`JM车号`、详情页链接。
- 支持完整本子和单独章节下载。
- 使用多线程并发下载：可分别设置图片并发和章节并发，批量车号也支持并行处理。
- 默认保持图片原格式，不缩小分辨率。
- JPEG 等格式可直接写入 PDF 时不进行二次有损压缩。
- PDF 生成后校验页数，校验成功才删除对应中间图片。
- PDF 失败时保留原图片，方便排查和重试。
- 浏览器扩展可扫描当前窗口的 JM 标签页，一行一个复制链接或车号。
- 提供系统代理、无代理和自定义本地代理设置。

## GUI 使用方法

1. 启动 `JMComic-Downloader-GUI.exe`。
2. 选择“本子”或“章节”。
3. 粘贴一个或多个车号/链接，支持空格、逗号、分号或换行分隔。
4. 选择保存位置。
5. 一般保持默认设置：
   - 保存位置：EXE 所在文件夹下的 `下载`
   - 客户端：移动端 API（推荐）
   - 图片格式：保持原格式
   - 图片并发：20
   - 章节并发：4
   - 代理：跟随系统
6. 点击“开始下载”，在右侧查看日志。
7. 完成后点击“打开 PDF 目录”。

输入示例：

```text
1455254
JM1455254
https://devapp.18comic.cc/comic/detail?id=1455254
```

当前“本子/章节”选择会应用于整批输入。本子与章节请分批下载。

## 新版 GUI（本地服务 + pywebview/WebView2 双窗口壳）

仓库同时提供一版全新桌面界面（`run_gui.py`，橙色主题），其核心是**在线预览**：在下载前即可搜索、浏览本子详情并在线阅读章节，全程只读、不写入下载目录。

运行方式（源码）：

```powershell
python run_gui.py            # 桌面窗口（pywebview + WebView2：主窗 + 独立阅读窗）
python run_gui.py --dev      # 开发模式：窗口调试 + 服务日志
python run_gui.py --browser  # 仅启动本地服务并在系统浏览器打开
```

> 桌面壳为 frameless pywebview 窗口（WebView2 渲染，自绘标题栏 + WndProc 子类化
> 拖动/缩放），依赖系统 **WebView2 Runtime**（Win10/11 + Edge 自带，一般无需安装）。
> WebView2 数据隔离：窗口数据落在 exe 同目录 `.webview2-cache`（专属 UserDataFolder），
> 不污染机器上其它 WebView2 实例（浏览器/其它应用）。

本地服务监听 `http://127.0.0.1:<随机端口>`，前端为自绘单页界面：

- 关键词 / 作者 / 车号搜索（纯数字自动直达车号详情）。
- 结果网格 → 本子详情（标签、页数、章节列表）→ 章节阅读器，支持懒加载翻页、上一话/下一话与键盘快捷键。
- **独立阅读窗口**（详情页「在新窗口阅读」）：真双窗，大画布沉浸阅读；标题栏显示专辑名与当前章节位置；左侧章节抽屉（默认收起）；下载按钮与主窗任务面板共享。
- **图片放大查看**：阅读页点击图片进入深色全屏放大层（滚轮缩放 1–8×、拖拽平移、双击复位、Esc/× 关闭）。
- 图片由本地服务代理解码（旧作品切片图内存重排），封面与章节页均不落盘。
- 下载任务、历史与设置等视图沿用相同视觉体系；下载能力迁移与本仓库原有 PDF 导出一致。

## PDF 与画质

- 不会为了减小文件体积主动降低 JPEG 质量或缩小分辨率。
- WebP 等不能直接嵌入当前 PDF 流程的格式，会按解码后的像素写入，因此 PDF 可能明显变大。
- 单章节本子生成 `本子名.pdf`。
- 多章节本子生成 `本子名 - 第N话 章节名.pdf`。
- 每个章节 PDF 页数校验成功后，才删除该章节的中间图片。

“几乎无损”不代表原始站点图片本身是无损格式，也不代表生成后的 PDF 一定更小。

## 浏览器扩展

扩展目录：`browser-extension`。

### Edge

1. 打开 `edge://extensions/`。
2. 开启“开发人员模式”。
3. 点击“加载解压缩的扩展”。
4. 选择解压后的 `browser-extension` 文件夹。
5. 在工具栏固定扩展。

### Chrome

1. 打开 `chrome://extensions/`。
2. 开启“开发者模式”。
3. 点击“加载已解压的扩展程序”。
4. 选择 `browser-extension` 文件夹。

扩展只在本地读取当前窗口标签页标题和 URL，用于识别 `/comic/detail?id=车号`、`/album/车号`、`/photo/车号` 等地址并复制；不会主动上传标签页数据。

## 从源码运行

要求：Windows、Python 3.10 或更高版本。

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install -e ".[test]"
.\.venv\Scripts\python.exe .\jmcomic_gui.py
```

## 构建 Windows EXE

仓库提供两套界面，各自独立构建：

**① 新版桌面界面（`run_gui.py`，pywebview + WebView2 双窗口 + 在线预览 + 下载任务，推荐）：**

```powershell
.\build_gui_exe.ps1            # 单 exe 分发（默认）
.\build_gui_exe.ps1 -IncludeWorker   # 可选：旧双 exe 布局
```

单 exe（默认）产物只有一个文件，**单独一个 exe 即可运行**：

```text
dist\gui\JMComic-Downloader-GUI.exe   主程序（--windowed，桌面窗口，内嵌下载 worker）
```

- 双击即用，无 Python 依赖；需系统 **WebView2 Runtime**（Win10/11 + Edge 自带）；
  exe 首次运行会自动解压内置运行时到系统临时目录（用后清理），并在 exe 同目录
  生成下载输出目录与 `.webview2-cache`（WebView2 专属数据目录）；
- 下载任务由主程序以 `--jm-worker` 参数自我拉起为隐藏子进程执行，无需旁挂文件；
- 加 `-IncludeWorker` 会额外产出 `JMComic-Downloader-Worker.exe`（旧双 exe 布局，
  下载子进程独立 exe、启动更快、内存占用更小；两个 exe 须保持同目录）。

**② 原版 GUI（`jmcomic_gui.py`，tkinter）：**

```powershell
.\build_exe.ps1
```

构建结果：`JMComic-Downloader-GUI.exe`

两个构建脚本都会安装对应依赖（① 安装 `requirements.txt` 含 pywebview/pythonnet —— 渲染走系统 WebView2 Runtime（Edge 自带），无需打包 Chromium；② 安装 `.[build]`）并调用 PyInstaller 打包。Release 构建前应运行测试并更新第三方许可证声明。

## 测试

GUI/PDF 离线测试：

```powershell
python -m unittest tests.test_jmcomic_gui_pdf   # 原有 GUI/PDF 导出测试
python -m unittest tests.test_jm_api            # 新版 GUI 在线预览 API 测试
python -m unittest tests.test_jobs_api          # 下载任务管理 / 历史持久化 / 队列统计
```

浏览器扩展测试：

```powershell
cd browser-extension
npm test
```

仓库中的 GitHub Actions 只执行上述离线测试，不包含云端漫画下载、收藏夹导出或 PyPI 发布。

## 项目结构

```text
JMComic-Downloader-GUI/
├─ jmcomic_gui.py             # 原版 Windows GUI 与 PDF 导出
├─ run_gui.py                 # 新版 GUI 桌面入口（本地服务 + pywebview 双窗口壳）
├─ gui/                       # 本地 HTTP 服务、预览 API 与桌面壳
├─ static/                    # 新版 GUI 前端（HTML/CSS/JS，橙色主题）
├─ src/jmcomic/               # 上游核心及本地调整
├─ browser-extension/         # Edge/Chrome 标签页复制器
├─ tests/                     # 离线 GUI/PDF / 预览 API / 任务管理测试
├─ build_exe.ps1              # 原版 GUI Windows EXE 构建
├─ build_gui_exe.ps1          # 新版 GUI + 下载 Worker 双 EXE 构建
├─ LICENSE                    # 保留的上游 MIT License
├─ UPSTREAM.md                # 上游来源和衍生关系
└─ THIRD_PARTY_NOTICES.md     # 第三方依赖声明
```

## 隐私、安全与法律

- 不要在 Issue 中上传账号、密码、Cookie、Token、完整个人路径、下载文件或成人内容截图。
- 默认保存位置始终跟随当前 EXE，使用其所在文件夹下的 `下载`；手动选择的自定义位置会继续保留。
- 设置保存在 `%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json`，首次运行会兼容读取旧版 `%LOCALAPPDATA%\JMComicDownloader\settings.json`。
- 本仓库不托管、不提供任何漫画或媒体内容。
- 请仅下载你有权访问和保存的内容，并遵守所在地法律、网站条款和版权要求。

详情见 [SECURITY.md](SECURITY.md) 与 [DISCLAIMER.md](DISCLAIMER.md)。

## 开发辅助工具

本项目开发过程中使用了 `gpt5.6-sol` 作为开发辅助工具。

## 许可证

本项目保留上游 MIT License 与以下原始版权声明：

```text
Copyright (c) 2023 hect0x7
```

完整许可证见 [LICENSE](LICENSE)。修改与新增部分的说明见 [NOTICE](NOTICE) 和 [UPSTREAM.md](UPSTREAM.md)。第三方依赖见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
