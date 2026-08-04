# JMComic-Downloader-GUI

Windows 图形化批量下载器：支持车号/链接输入、无额外有损的 PDF 整合、校验后自动清理中间图片，以及 Chromium 浏览器标签页批量收集。

> 本项目是 `hect0x7/JMComic-Crawler-Python` 的非官方衍生版本，不属于 JMComic、18comic 或上游作者的官方发行版。上游来源与修改说明见 [UPSTREAM.md](UPSTREAM.md)。

## 下载

普通 Windows 用户无需安装 Python。请前往 GitHub 仓库的 **Releases** 页面，下载：

```text
JMComic-Downloader-GUI-1.2.0-Windows-x64.zip
```

下载后完整解压，再运行：

```text
JMComic-Downloader-GUI.exe
```

Release 同时提供 `.sha256.txt` 校验文件。Windows SmartScreen 或杀毒软件可能对未签名的 PyInstaller 程序显示提示；请只从正式 Release 下载并核对 SHA256，不要关闭杀毒软件。

## 主要功能

- Windows 10/11 图形界面，无需命令行。
- 支持单个或批量输入 JM 车号、`JM车号`、详情页链接。
- 支持完整本子和单独章节下载。
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
https://example.com/album/1455254
```

当前“本子/章节”选择会应用于整批输入。本子与章节请分批下载。

## PDF 与画质

- 不会为了减小文件体积主动降低 JPEG 质量或缩小分辨率。
- WebP 等不能直接嵌入当前 PDF 流程的格式，会按解码后的像素写入，因此 PDF 可能明显变大。
- 单章节本子生成 `本子名.pdf`。
- 多章节本子生成 `本子名 - 第N话 章节名.pdf`。
- 每个章节 PDF 页数校验成功后，才删除该章节的中间图片。

“无额外有损”不代表原始站点图片本身是无损格式，也不代表生成后的 PDF 一定更小。

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

```powershell
.\build_exe.ps1
```

构建结果：

```text
JMComic-Downloader-GUI.exe
```

构建脚本会安装 `.[build]` 依赖并使用 PyInstaller 打包。Release 构建前应运行测试并更新第三方许可证声明。

## 测试

GUI/PDF 离线测试：

```powershell
python -m unittest tests.test_jmcomic_gui_pdf
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
├─ jmcomic_gui.py             # Windows GUI 与 PDF 导出
├─ src/jmcomic/               # 上游核心及本地调整
├─ browser-extension/         # Edge/Chrome 标签页复制器
├─ tests/                     # 离线 GUI/PDF 测试
├─ build_exe.ps1              # Windows EXE 构建
├─ LICENSE                    # 保留的上游 MIT License
├─ UPSTREAM.md                # 上游来源和衍生关系
└─ THIRD_PARTY_NOTICES.md     # 第三方依赖声明
```

## 隐私、安全与法律

- 不要在 Issue 中上传账号、密码、Cookie、Token、完整个人路径、下载文件或成人内容截图。
- 设置保存在 `%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json`，首次运行会兼容读取旧版 `%LOCALAPPDATA%\JMComicDownloader\settings.json`。
- 本仓库不托管、不提供任何漫画或媒体内容。
- 请仅下载你有权访问和保存的内容，并遵守所在地法律、网站条款和版权要求。

详情见 [SECURITY.md](SECURITY.md) 与 [DISCLAIMER.md](DISCLAIMER.md)。

## 许可证

本项目保留上游 MIT License 与以下原始版权声明：

```text
Copyright (c) 2023 hect0x7
```

完整许可证见 [LICENSE](LICENSE)。修改与新增部分的说明见 [NOTICE](NOTICE) 和 [UPSTREAM.md](UPSTREAM.md)。第三方依赖见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
