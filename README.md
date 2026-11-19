# JMComic Downloader

Windows 桌面批量下载器：**Rust 原生实现、单个 EXE、无需 Python 运行时**。支持车号/链接批量输入、多级并发下载、近乎无损的 PDF 整合、自动清理中间图片，并附带 Chromium 浏览器标签页收集扩展。

> 本项目是对开源项目 `hect0x7/JMComic-Crawler-Python` 的衍生实现：界面与下载业务最初为 Python 版本，现已整体重写为 Rust，仅保留 Rust 代码。上游来源、MIT License 与原作者版权声明见 [UPSTREAM.md](UPSTREAM.md)；不属于 JMComic、18comic 或上游作者的官方发行版。

## 性能

同一台机器、同一批图片（1200×1700 JPEG，取 30 次平均，Rust 使用 mimalloc 分配器）：

| 场景 | Python(Pillow) | Rust | 提升 |
| --- | --- | --- | --- |
| 保存一张图（无需解密、保持原格式） | 16.4 ms | **0.67 ms** | ≈ 24× |
| 竖切解密 + WebP 无损重编码 | 2103 ms | **23.5 ms** | ≈ 89× |
| 20 页 JPEG 生成 PDF（直嵌不重编码） | 30.1 ms | **23.2 ms** | ≈ 1.3× |

除单图处理外，下载链路本身也做了优化：

- **三级并发**：本子 / 章节 / 图片分别限流，异步 I/O 无线程栈与 GIL 开销；默认并发 4 / 8 / 32，可调到 32 / 64 / 128；
- **图片快速直写**：无需解密且格式未变时直接写原始字节，跳过解码与重编码；
- **请求路径零分配**：预构建请求头、域名列表用 `Arc` 共享、按图片序号把流量分摊到多个 CDN；
- **CPU/IO 分离**：解码、竖切、编码、PDF 写入放在 `spawn_blocking`，并单独限制 PDF 并发，避免磁盘抖动；
- **日志批量刷新**：核心日志以 100 ms 批次刷新到界面，高频日志不会拖慢 UI。

Rust 侧可用 `examples/pipeline_bench.rs` 复现（`cargo run --release --example pipeline_bench`）；表中的 Python 数据来自旧的 1.2.0 实现，该实现已从仓库移除。

## 界面

- 跟随 Windows 11 的**浅色/深色模式与系统强调色**（读取注册表，可在标题栏切换“跟随系统 / 浅色 / 深色”）；
- 圆角卡片、细分隔线、Segoe UI + 等线中文字体，适配 100%–200% 缩放；
- 实时日志（可选中复制）、进度条、下载速度、已用时间、状态指示、Ctrl+Enter 快捷开始；
- 停止任务为协作式取消，正在传输的请求收尾后退出，已下载图片保留可续传。

## 快速开始

要求：Windows 10/11、Rust 1.82+（MSVC 工具链）。

```powershell
cargo build --release
.\target\release\jmcomic-downloader-gui.exe
```

一键构建（复制 EXE 到仓库根目录）：

```powershell
.\build_exe.ps1
```

发布包（ZIP + SHA256 + 使用说明 + 浏览器扩展）：

```powershell
.\build_release.ps1
# 产物：dist\JMComic-Downloader-GUI-<版本>-Windows-x64.zip
```

## 使用

### 图形界面

双击 EXE（或 `启动GUI.bat`）。输入车号或链接，选择保存位置，点击“开始下载”。

```text
1455254
JM1455254
https://devapp.18comic.cc/comic/detail?id=1455254
```

支持空格、逗号、分号、换行分隔的批量输入。默认保存到 EXE 所在目录下的 `下载`，PDF 输出到其中的 `PDF` 子目录；PDF 页数校验通过后才会清理对应章节的中间图片。

### 命令行

```powershell
# 下载整本
.\jmcomic-downloader-gui.exe --cli --kind album --id 1455254 --output D:\Comics

# 批量章节 + 限制并发
.\jmcomic-downloader-gui.exe --cli --kind photo --id 1455254 --id JM1455255 `
    --image-threads 16 --photo-threads 4 --album-threads 2

# 代理与自定义域名
.\jmcomic-downloader-gui.exe --cli --kind album --id 1455254 `
    --proxy 127.0.0.1:7890 --api-domain www.cdngwc.cc --image-domain cdn-msp2.jmapiproxy2.cc
```

完整参数：

```text
--cli                      不启动图形界面，直接执行下载
--self-test                离线自检（加密、图片解密、PDF 生成与校验，不联网）
--kind <album|photo>       本子 / 章节
--id <ID>                  车号或链接，可重复；单个参数内也可分隔多个
--output <DIR>             保存位置
--client <api|html>        移动端 API（默认）/ 网页端 HTML
--image-format <original|jpg|png|webp>
--image-threads <N>        图片并发（1-128）
--photo-threads <N>        章节并发（1-64）
--album-threads <N>        本子并发（1-32，批量车号）
--proxy <system|none|host:port>
--option <FILE>            高级 YAML 配置
--log-file <FILE>          额外写入日志文件
--api-domain <DOMAIN>      自定义接口域名（可重复，支持 host / host:port / 完整 URL）
--image-domain <DOMAIN>    自定义图片 CDN 域名（可重复）
```

退出码：`0` 全部成功；`2` 存在失败项（详见日志）；`1` 致命错误；`64` 参数错误。

### 自检

```powershell
.\jmcomic-downloader-gui.exe --self-test
```

## 架构

```text
src/
├─ main.rs        CLI 参数解析、无界面执行、GUI 启动
├─ lib.rs         模块树
├─ config.rs      常量：域名、密钥、请求头、并发上限
├─ crypto.rs      token 计算、AES-256-ECB 响应体解密
├─ http.rs        HTTP 客户端（连接池 / Cookie / 系统代理）
├─ api.rs         域名轮换、重试、详情接口、图片下载
├─ html_client.rs 网页端正则解析（本子页 / 章节页 / 域名列表）
├─ types.rs       Album / Photo 实体
├─ naming.rs      车号解析、Windows 文件名清洗、PDF 命名
├─ imageio.rs     竖切解密、格式转换、快速直写、JPEG/EXIF 解析
├─ pdf.rs         PDF 生成与页数校验
├─ download.rs    三级并发编排、进度统计、清理
├─ option.rs      YAML 配置子集
├─ settings.rs    设置持久化
├─ win_theme.rs   Windows 主题 / 强调色检测
├─ selftest.rs    离线自检
└─ gui.rs         egui 图形界面
tests/mock_download.rs   本地模拟 API 的端到端测试（真实 AES 响应 + 真实 JPEG）
examples/                图片管线与 PDF 基准、PDF 回环校验
```

下载流水线：

```text
车号 → 本子详情 ─┬→ 章节详情 ─┬→ 图片并发下载 → 解密/转码 → 落盘
                 │            ├→ …（章节并发）
                 │            └→ 全部成功 → 生成 PDF → 校验页数 → 删除中间图片 → 清理空目录
                 └→ …（本子并发）
```

## 高级 YAML 配置

支持与下载相关的配置子集：

```yaml
dir_rule:
  base_dir: D:/Comics
download:
  cache: true
  image:
    decode: true
    suffix: .webp
  threading:
    image: 64
    photo: 12
client:
  impl: api
  retry_times: 3
  postman:
    meta_data:
      proxies: 127.0.0.1:7890
```

`plugins` 等未支持的配置会打印提示后忽略，不会静默失败。

## 与早期 Python 版的差异

| 项目 | 说明 |
| --- | --- |
| Python 代码 | 已全部移除；仓库只保留 Rust 实现 |
| 插件（plugins） | 未迁移。YAML 中的 `plugins` 会提示后忽略 |
| 繁简转换（zhconv） | 未迁移，目录名保持原始文本 |
| 搜索 / 收藏 / 登录 / 评论 | 未迁移，只覆盖“下载”链路 |
| 设置文件 | 路径与字段兼容旧版：`%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json` |
| 停止语义 | 协作式取消，不再使用 `taskkill` 杀进程树 |
| PDF | JPEG 以 `DCTDecode` 原样嵌入；其他格式 `FlateDecode` 无损嵌入；96 DPI 换算与 `img2pdf` 一致 |

## 测试

```powershell
cargo test
```

- 51 个单元测试：加密/解密、车号解析、文件名清洗、HTML 解析、图片解密与格式转换、PDF 结构与页数、设置迁移、主题解析、清理安全性、YAML 子集；
- 2 个端到端测试：本地 TCP 服务器模拟禁漫 API（真实 AES 加密响应、真实 JPEG），覆盖“请求 → 解密 → 解析 → 下载 → PDF → 清理”与失败记录；
- `--self-test`：不联网的冒烟测试，发布前由 `build_release.ps1` 自动执行。

浏览器扩展测试：

```powershell
cd browser-extension
npm test
```

## 项目结构

```text
JMComic-Downloader-GUI/
├─ src/                  Rust 源码
├─ tests/                端到端测试
├─ examples/             基准与 PDF 回环校验
├─ browser-extension/    Edge/Chrome 标签页复制扩展
├─ docs/                 使用说明与迁移说明
├─ Cargo.toml            Rust 工程配置
├─ build_exe.ps1         构建 EXE
├─ build_release.ps1     构建发布包（ZIP + SHA256）
├─ LICENSE               保留的上游 MIT License
└─ UPSTREAM.md           上游来源和衍生关系
```

## 隐私、安全与法律

- 不要在 Issue 中上传账号、密码、Cookie、Token、完整个人路径、下载文件或成人内容截图。
- 默认保存位置始终跟随当前 EXE，使用其所在文件夹下的 `下载`；手动选择的自定义位置会继续保留。
- 设置保存在 `%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json`。
- 本仓库不托管、不提供任何漫画或媒体内容。
- 请仅下载你有权访问和保存的内容，并遵守所在地法律、网站条款和版权要求。

详见 [SECURITY.md](SECURITY.md) 与 [DISCLAIMER.md](DISCLAIMER.md)。

## 许可证

本项目保留上游 MIT License 与以下原始版权声明：

```text
Copyright (c) 2023 hect0x7
```

完整许可证见 [LICENSE](LICENSE)。第三方依赖见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。