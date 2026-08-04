# 上游来源与版权说明

本项目 `JMComic-Downloader-GUI` 是基于以下开源项目制作的衍生版本：

- 上游项目：`JMComic-Crawler-Python`
- 上游作者：`hect0x7`
- 上游仓库：`https://github.com/hect0x7/JMComic-Crawler-Python`
- 上游许可证：MIT License
- 本项目使用的上游核心版本：`jmcomic 2.7.2`

仓库根目录中的 `LICENSE` 完整保留了上游项目原始版权声明：

```text
Copyright (c) 2023 hect0x7
```

原始上游 README 的本地副本位于 `docs/UPSTREAM_README.md`，用于说明来源和保留历史文档。上游项目名称、商标、网站及服务均不由本项目维护者拥有。

## 本衍生版本的主要新增或调整

- Windows Tkinter 图形界面。
- 批量输入车号或链接。
- 下载后按章节生成 PDF。
- 不缩放图片、不主动降低 JPEG 质量的 PDF 转换流程。
- PDF 页数校验通过后清理对应中间图片。
- Chromium 浏览器标签页批量收集扩展。
- Windows 独立 EXE 构建脚本。
- 与 GUI/PDF 逻辑对应的离线测试。
- 对上游同步与异步下载实现的本地性能和稳定性调整。

## 兼容性说明

源码中的 Python 导入包仍名为 `jmcomic`。这是为了与上游 API 和内部导入路径保持兼容；项目发行名称、GitHub 仓库名称和 Windows 程序名称均为 `JMComic-Downloader-GUI`。

本仓库不会以 `jmcomic` 名称发布到 PyPI，也不会声称是上游官方发行版。
