# 第三方软件声明

`JMComic-Downloader-GUI` 包含或依赖多个第三方开源项目。各项目仍由其各自作者持有版权，并适用各自许可证。本文件不是许可证全文的替代。

## 核心来源

| 项目 | 用途 | 许可证/说明 |
| --- | --- | --- |
| JMComic-Crawler-Python / jmcomic 2.7.2 | 下载与解析核心 | MIT；原始版权和许可证见 `LICENSE`、`UPSTREAM.md` |
| commonX 0.6.40 | 上游核心通用工具 | MIT；Copyright (c) 2023 hect0x7 |

## 主要运行时依赖

| 项目 | 构建时版本 | 主要用途 | 许可证 |
| --- | ---: | --- | --- |
| curl_cffi | 0.16.0 | HTTP 客户端 | MIT |
| Pillow | 12.3.0 | 图片读取和格式处理 | MIT-CMU |
| PyCryptodome | 3.23.0 | 加解密支持 | BSD / Public Domain |
| PyYAML | 6.0.3 | YAML 配置解析 | MIT |
| img2pdf | 0.6.3 | 图片无额外有损地写入 PDF | LGPL-3.0-or-later |
| pypdf | 6.14.2 | PDF 页数校验 | BSD-3-Clause |
| Requests | 2.32.5 | 高级 YAML 的可选 HTTP 后端兼容 | Apache-2.0 |
| certifi | 2026.7.22 | CA 证书集合 | MPL-2.0 |
| cffi | 2.1.0 | curl_cffi 的 FFI 支持 | MIT-0 |
| pycparser | 3.0 | cffi 解析支持 | BSD-3-Clause |
| charset-normalizer | 3.4.9 | Requests 字符编码处理 | MIT |
| idna | 3.18 | Requests 国际化域名处理 | BSD-3-Clause |
| urllib3 | 2.7.0 | Requests HTTP 连接层 | MIT |

## 打包运行时

Windows v1.2.0 EXE 还包含 Python 3.12.7、Tcl/Tk 8.6.14，以及构建脚本显式加入的 bzip2、libffi、liblzma/xz 和 Expat 运行库。对应许可证副本保存在仓库 `third_party_licenses/`，并会复制到 Release 包的 `licenses/` 目录。

## 构建工具

| 项目 | 构建时版本 | 许可证 |
| --- | ---: | --- |
| PyInstaller | 6.21.0 | GPL-2.0-or-later，附带允许分发所构建应用程序的特别例外 |

GitHub 源码仓库不会提交由上述依赖生成的虚拟环境或构建缓存。Windows Release 包会在 `licenses/` 目录中附带构建环境可取得的主要依赖许可证副本。

如果依赖版本发生变化，请在发布新版本前重新核对并更新此文件。
