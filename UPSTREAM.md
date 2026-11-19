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

- 整体重写为 Rust：单 EXE、异步多级并发、自研 PDF 生成，仓库不再包含 Python 代码。
- Windows 图形界面（egui）：跟随系统浅色/深色与强调色，实时日志、进度与速度显示。
- 批量输入车号或链接。
- 下载后按章节生成 PDF。
- 不缩放图片、不主动降低 JPEG 质量的 PDF 转换流程。
- PDF 页数校验通过后清理对应中间图片。
- Chromium 浏览器标签页批量收集扩展。
- Windows 独立 EXE 构建与发布打包脚本。
- 与下载、图片解密、PDF 逻辑对应的单元测试与端到端测试。

## 兼容性说明

Rust 版保留了上游的协议细节：请求 `token`/`tokenparam` 计算、AES-256-ECB 响应解密、图片竖切规则、目录与文件名规则、`settings.json` 字段等，便于与旧版设置和行为对照。

本仓库不会以 `jmcomic` 名称发布到 PyPI，也不会声称是上游官方发行版。
