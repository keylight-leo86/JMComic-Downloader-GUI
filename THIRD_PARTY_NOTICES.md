# 第三方软件声明

`JMComic Downloader` 为 Rust 原生实现，包含或依赖多个第三方开源项目。各项目仍由其各自作者持有版权，并适用各自许可证。本文件不是许可证全文的替代。

## 核心来源

| 项目 | 用途 | 许可证/说明 |
| --- | --- | --- |
| JMComic-Crawler-Python / jmcomic 2.7.2 | 协议、解析规则与业务逻辑的上游来源 | MIT；原始版权和许可证见 `LICENSE`、`UPSTREAM.md` |

Rust 版按上游协议与算法重新实现（请求签名、AES 响应解密、图片竖切、目录规则等），不再直接依赖上游 Python 代码。

## 运行时依赖

| crate | 用途 | 许可证 |
| --- | --- | --- |
| tokio | 异步运行时 | MIT |
| reqwest（rustls 后端） | HTTP 客户端 | MIT / Apache-2.0 |
| rustls / ring / webpki-roots | TLS 与根证书 | ISC / Apache-2.0 / MIT / MPL-2.0 |
| serde / serde_json / serde_yaml | 序列化与配置 | MIT / Apache-2.0 |
| md-5 / aes / cipher | MD5 与 AES-256-ECB | MIT / Apache-2.0 |
| base64 | Base64 编解码 | MIT / Apache-2.0 |
| image（jpeg/png/webp/gif） | 图片编解码 | MIT / Apache-2.0 |
| flate2（miniz_oxide） | PDF 的 zlib 压缩 | MIT / Apache-2.0 |
| regex | 网页端正则解析 | MIT / Apache-2.0 |
| unicode-normalization | 文件名 NFKC 归一化 | MIT / Apache-2.0 |
| eframe / egui | 图形界面 | MIT / Apache-2.0 |
| rfd | 系统文件对话框 | MIT |
| clap | 命令行解析 | MIT / Apache-2.0 |
| chrono | 日志时间戳 | MIT / Apache-2.0 |
| anyhow / thiserror | 错误处理 | MIT / Apache-2.0 |
| mimalloc | 高性能内存分配器 | MIT |

## 已移除的依赖

2.0.0 起，Python 运行时与以下依赖不再随项目分发：Python 3.12、Tcl/Tk、commonX、curl_cffi、Pillow、img2pdf、pypdf、PyCryptodome、PyYAML、Requests、certifi 等。PDF 由本项目自行生成，因此也不再涉及 `img2pdf` 的 LGPL-3.0-or-later 许可。

git 历史中仍可查到 1.2.0 时期的 `third_party_licenses/` 目录。

## 浏览器扩展

`browser-extension/` 为纯 JavaScript（Manifest V3），不包含第三方运行时依赖。

## 核对方式

如需逐 crate 核对许可证，可在仓库根目录运行 `cargo tree`，或安装 `cargo-license` 后执行 `cargo license`。