# Changelog

本项目遵循语义化版本号。

## 2.1.0 - 2026-11-19

界面可读性改进。

- 新增界面缩放设置：标题栏“缩放”可选 100% / 110% / 125% / 150%，取值保存到 `settings.json` 的 `ui_scale`。
- 缩放基准调整：界面上的 100% 即原来的 125% 观感（`UI_BASE_SCALE = 1.25`），默认即使用该字号。
- 新增快捷键：`Ctrl + =` 放大、`Ctrl + -` 缩小（步进 5%，范围 80%–200%）、`Ctrl + 0` 复位 100%。
- 默认字号整体加大：正文 13→14、节标题 13→14、日志 12→12.5、等宽字体 13.5，窗口默认尺寸 1200×880。
- 窄窗口（高缩放）自适应：侧栏宽度按比例收窄、底栏按钮自动换行。
- 修复 `启动GUI.bat` 使用 LF 换行导致 `cmd.exe` 解析失败、路径碎片被当成参数的问题（改为 CRLF + `goto` 标签，并增加 `dist\` 回退）。
- 新增 `.gitattributes` 语言统计规则（文档不计入语言占比），修复 GitHub 语言条长期显示旧 Python 占比的问题。

## 2.0.0 - 2026-11-19

完整重写为 Rust：仓库只保留 Rust 实现、浏览器扩展与文档，Python 代码、依赖和打包产物全部移除。

### 下载核心

- 移动端 API / 网页端 HTML 双客户端，域名轮换与重试策略与上游一致。
- 三级并发：本子 / 章节 / 图片分别限流，异步 I/O；默认 4 / 8 / 32，可调到 32 / 64 / 128。
- 图片处理：无需解密且格式未变时直接写原始字节；需要解密时只解码、编码各一次（JPEG 质量 95、WebP 无损）。
- 自研 PDF 生成器：JPEG 以 `DCTDecode` 原样嵌入，其它格式 `FlateDecode` 无损嵌入；生成后校验页数，校验通过才清理中间图片。
- 失败隔离：单图失败只记录该图，章节跳过 PDF 并保留已下载图片；单章失败不影响整批。
- 停止改为进程内协作式取消，不再使用 `taskkill` 杀进程树。

### 性能

- 引入 mimalloc 分配器（同一图片管线约 +15%）。
- 请求头预构建、域名列表 `Arc` 共享、图片按序号分摊到多个 CDN。
- PDF 生成限制为 2 个并发，避免顺序读盘造成磁盘抖动。
- 日志 100 ms 批量刷新，高频日志不再拖慢界面。
- 实测（1200×1700 JPEG，30 次平均）：免解密直写 0.67 ms/张、竖切解密 + WebP 无损 23.5 ms/张、20 页 PDF 23.2 ms。

### 界面（Windows 11）

- egui 重写界面：跟随系统浅色/深色模式与系统强调色，可在标题栏切换“跟随系统 / 浅色 / 深色”并持久化。
- Segoe UI + 等线/黑体 + Consolas 字体、圆角卡片、细分隔线，适配高 DPI。
- 实时日志（可选中复制）、进度条、下载速度、已用时间、状态指示、Ctrl+Enter 快捷开始。
- 运行时可生成应用图标，任务栏使用固定 AppID 分组。

### 工程与发布

- 仓库结构迁移到根目录：`Cargo.toml`、`src/`、`tests/`、`examples/`；移除 `jmcomic_gui.py`、`src/jmcomic/`、`pyproject.toml`、`requirements*.txt`、`.venv/`、`third_party_licenses/` 等 Python 内容。
- 新增命令行模式（`--cli`）、离线自检（`--self-test`）、自定义接口/CDN 域名（`--api-domain` / `--image-domain`）。
- 支持 YAML 配置子集（`dir_rule` / `download` / `client`）；插件、搜索、收藏、繁简转换未迁移并会给出明确提示。
- 设置文件保持兼容：`%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json`，新增 `theme` 字段。
- 测试：51 个单元测试 + 2 个本地模拟 API 的端到端测试；`build_release.ps1` 在打包前自动运行测试与自检。
- 构建：`build_exe.ps1` / `build_release.ps1`（ZIP + SHA256SUMS），产物约 12.5 MB。
- 新增 `docs/RUST_MIGRATION.md`，记录模块映射、算法等价性、并发模型、性能数据与踩坑记录。
- CI 改为 Rust（fmt / clippy / test / self-test / release build）+ 浏览器扩展测试。

## 1.2.0 - 2026-08-04

- 建立独立项目名称 `JMComic-Downloader-GUI`。
- 提供 Windows Tkinter 图形界面和批量输入。
- 下载后按章节生成 PDF。
- PDF 转换不缩小图片、不主动降低 JPEG 质量。
- PDF 页数校验成功后自动删除对应中间图片。
- PDF 失败时保留原图片。
- 提供 Edge/Chrome 标签页复制扩展 1.0.2。
- 增加代理、并发和图片格式设置。
- 默认下载目录跟随当前 EXE 所在文件夹，并迁移旧版默认路径设置。
- 在说明文档中增加 `gpt5.6-sol` 开发辅助工具声明。
- 增加 GUI/PDF 离线测试和浏览器扩展测试。
- 保留上游 MIT License 和原作者版权声明。
- 移除上游 PyPI 发布、云端内容下载和收藏夹导出工作流。
