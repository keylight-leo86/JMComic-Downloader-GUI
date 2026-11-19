# Rust 迁移说明

本文记录 Python 版（原 `jmcomic_gui.py` + `src/jmcomic/`）到 Rust 版（当前 `src/`）的迁移范围、算法等价性、并发模型、性能数据与维护要点。

## 1. 目标与结论

目标：在功能对齐的前提下，把下载、解密、PDF 整合从“Python 解释器 + 多线程 + 子进程”迁移为“单个原生 EXE + 异步任务”，提升吞吐与并发上限。

结论：

- 下载链路（本子 / 章节 / 图片 / PDF / 清理）已完整迁移，含移动端 API 与网页端 HTML 两种客户端；
- 图片管线在相同输入下与 Pillow 版逐像素等价（竖切重排算法一致），并修复了 Python 版“无需解密也会重编码”的性能问题；
- PDF 语义与 `img2pdf` 对齐：JPEG 直嵌不重编码，页面尺寸按 96 DPI 换算；
- 旧 Python 源码与运行时依赖已从仓库移除，git 历史中仍可追溯；
- 未迁移：插件系统、搜索/收藏/登录、繁简转换（见第 6 节）。

## 2. 模块映射

| Python | Rust | 说明 |
| --- | --- | --- |
| `jmcomic_gui.py`（GUI/流程） | `main.rs` + `gui.rs` + `download.rs` | GUI 与编排拆分，CLI 与 GUI 共用同一核心 |
| `jm_config.py`（常量/默认值） | `config.rs` | 域名、密钥、请求头、并发默认值 |
| `JmCryptoTool` | `crypto.rs` | token 计算、AES-256-ECB 响应解密（另含测试用加密） |
| `AbstractJmClient` / `JmApiClient` / `JmHtmlClient` | `api.rs` | 域名轮换、重试、详情接口、图片下载 |
| `JmcomicText` / `JmPageTool`（HTML） | `html_client.rs` | 正则原样移植 + `OnceLock` 缓存编译结果 |
| `JmAlbumDetail` / `JmPhotoDetail` | `types.rs` | 实体与图片 URL/文件名规则 |
| `JmImageTool` | `imageio.rs` | 竖切解密、格式转换、快速直写、JPEG/EXIF 解析 |
| `img2pdf` + `pypdf` 校验 | `pdf.rs` | 自研 PDF writer + 页数校验 |
| `JmDownloader.execute_on_condition` | `download.rs` | 三级并发、失败隔离、清理 |
| `common.ProxyBuilder` | `http.rs` | 环境变量 + Windows 注册表读取系统代理 |
| GUI 子进程 worker | 进程内 tokio 任务 | 不再需要 `--worker` 子进程与 `taskkill` |
| Tkinter 界面 | `gui.rs` + `win_theme.rs` | egui 界面、Win11 主题与强调色 |

## 3. 关键算法等价性

### 3.1 请求签名与响应解密

与上游完全一致：

```text
tokenparam = "{ts},{APP_VERSION}"
token      = md5("{ts}{secret}")            # secret 默认 185Hcomic3PAPP7R
key        = md5("{ts}{secret}").as_bytes()  # 32 字节，AES-256-ECB
plain      = AES-ECB-Decrypt(base64(data)) 去掉末尾 PKCS#7 填充
```

特殊接口 `/chapter_view_template` 使用 `18comicAPPContent` 作为 secret，返回值是 HTML 文本，因此不参与 JSON 校验。

### 3.2 图片竖切解密

`descramble_num()` 与上游 `JmImageTool.get_num()` 完全一致：

```text
aid < scramble_id                → 0      （不处理）
aid < 268850                     → 10
否则 x = (aid < 421926 ? 10 : 8)
     n = ord(md5(f"{aid}{stem}")[-1]) % x
     num = n * 2 + 2
```

重排规则（`descramble_rgb`）逐行复刻 `crop/paste` 循环，包括 `over = h % num` 的余数落在第 0 段这一细节；单元测试覆盖整除与非整除两种情形。

### 3.3 PDF

- 每张图片一页，页面尺寸 = 像素 × 72/96（与 `img2pdf` 默认 96 DPI 一致）；
- JPEG：`/Filter /DCTDecode` 直接嵌入原始字节；1/3/4 分量分别映射 `DeviceGray/DeviceRGB/DeviceCMYK`；
- 其它格式：解码为 RGB 后 `zlib` 压缩并以 `/FlateDecode` 嵌入（无损）；
- 支持 GIF 仅首帧、EXIF orientation → `/Rotate`；
- 写临时文件 → 重新解析统计页数 → 通过后原子替换目标文件，失败则删除临时文件。

页数校验扫描对象字典中的 `/Type /Page`（排除 `/Pages`），因为 PDF 由本项目生成，结构可控；`examples/pdf_roundtrip.rs` 可用 pypdf 等外部解析器做交叉验证。

### 3.4 文件命名与清理

- `parse_ids`：支持 `1455254` / `JM1455254` / `A1455254` / `P1455254` / `/album/xxx` / `?id=xxx`，去前导零并去重；
- `safe_pdf_stem`：NFKC 归一化 → 非法字符替换 → 空白折叠 → Windows 保留名加前缀 → 截断（默认 180，本子名 110）；
- 清理：仅删除保存根目录内、且不在 `PDF` 目录中的文件，随后自底向上删除空目录，不触碰根目录。

## 4. 并发模型

```text
JoinSet(每车号一个任务) —— Semaphore(album_concurrency)
   └─ JoinSet(每章节一个任务) —— Semaphore(photo_concurrency)
        └─ JoinSet(每图片一个任务) —— Semaphore(image_concurrency)
             └─ fetch_image（异步） + spawn_blocking（解码/重排/编码/写盘）
                  └─ 全部成功后 spawn_blocking 生成 PDF（受 pdf_sem 限制）
```

- 三级信号量避免“章节并发占满但图片并发空闲”的死角；
- CPU 密集段全部放进 `spawn_blocking`，避免阻塞 tokio worker；
- PDF 生成额外限制为 2 个并发，避免大量顺序读盘造成磁盘抖动；
- 失败隔离：单图失败只记录该图，章节跳过 PDF 并保留已下载图片；
- 取消：`StopFlag`（`AtomicBool`）在任务启动、图片下载前检查，停止后不再发起新请求。

## 5. 性能数据

同机同图片（1200×1700 JPEG，30 次平均，Rust 侧用 `examples/pipeline_bench.rs` 复现；Python 数据来自已移除的 1.2.0 实现）：

| 场景 | Python(Pillow 13) | Rust 2.0.0 | 说明 |
| --- | --- | --- | --- |
| 保存一张图（无需解密、格式不变） | 16.4 ms | 0.67 ms | Rust 直接写原始字节；Python 仍解码 + 重编码 |
| 竖切解密 + WebP 无损 | 2103 ms | 23.5 ms | 两者输出均为无损 WebP |
| 20 页 JPEG → PDF | 30.1 ms | 23.2 ms | 均为 DCT 直嵌，主要受写盘带宽限制 |

分配器对比（同一管线，`examples/pipeline_bench.rs`）：

| 分配器 | 竖切+WebP | 20 页 PDF |
| --- | ---: | ---: |
| 系统默认（Windows Heap） | 26.2 ms | 27.2 ms |
| mimalloc | 22.4 ms | 23.6 ms |

> 说明：单个 WebP 无损编码的差异主要来自编码器实现，不代表所有图片处理都快 90 倍；稳定收益来自“快速直写路径”、多级并发与更低的调度开销。

## 6. 未迁移的功能

| 功能 | 原因 | 影响 |
| --- | --- | --- |
| 插件系统（`plugins`） | Python 插件依赖动态导入与约定接口，逐插件重写成本高 | YAML 中出现 `plugins` 会提示后忽略 |
| 搜索 / 分类 / 收藏 / 登录 / 评论 | GUI 下载器不使用 | 不影响下载流程 |
| `normalize_zh`（zhconv 繁简转换） | 需要额外的繁简词表 | 目录名保持原始文本 |
| 自定义 `dir_rule` DSL（如 `Bd_Pname`） | Rust 固定为“本子标题 / 章节序号” | 通过 `base_dir` 仍可指定根目录 |
| 异步客户端（`jm_async_client`） | Rust 版本身就是异步实现 | 功能等价 |

## 7. 界面实现

- `win_theme.rs` 读取 `AppsUseLightTheme` 与 `AccentColor`（ABGR），解析失败时回退浅色 + 默认绿色；
- `gui.rs` 根据主题生成两套调色板，并按强调色亮度自动选择按钮前景色（浅色强调色用深色文字）；
- 字体：Segoe UI（西文）+ 等线/黑体（中文）+ Consolas（等宽），均从系统字体目录读取，不随包分发字体；
- 日志以 100 ms 批次刷新，避免高频日志触发整块文本重排；
- 任务在独立线程的 tokio 运行时执行，界面线程只做绘制与事件轮询。

> 踩坑记录：在 `ui.horizontal` 里给 `TextEdit` 设置 `desired_width(f32::INFINITY)` 会让侧栏内容宽度失控，进而把 `SidePanel` 的报告宽度撑大，界面上表现为侧栏右侧出现一条黑色空白带。改为按行宽计算固定宽度即可（见 `output_card` / `advanced_card`）。

## 8. 测试策略

1. 单元测试（51 个）：加密向量、车号解析、文件名清洗、HTML 正则解析、竖切重排、JPEG 头解析、PDF 结构与页数、设置迁移、主题解析、清理安全性、YAML 子集；
2. 端到端测试（2 个，`tests/mock_download.rs`）：本地 TCP 服务器模拟禁漫 API，返回**真实 AES 加密**的响应体与真实 JPEG，验证“重试 → 解密 → 解析 → 下载 → PDF → 清理”全链路，以及 404 车号的失败记录；
3. 离线自检（`--self-test`）：不联网验证加密、图片直写、竖切解密、PDF 生成与页数校验，可用于发布包冒烟测试。

## 9. 维护指引

- **域名失效**：`config.rs` 的 `DEFAULT_API_DOMAINS` / `DEFAULT_IMAGE_DOMAINS` 可直接更新；运行时也会尝试从 `API_DOMAIN_SERVER_LIST` 自动拉取最新 API 域名，并支持 `--api-domain` 手动指定；
- **网页端改版**：只需调整 `html_client.rs` 中对应的正则，测试用 HTML fixture 位于同文件的 `#[cfg(test)]` 模块；
- **接口字段变化**：`api.rs` 的 `album_from_json` / `photo_from_json` 为纯函数，容易按新结构修改；
- **并发上限**：`config.rs` 的 `MAX_*_THREADS` 与默认值集中管理，GUI/CLI 共用；
- **依赖升级**：`Cargo.toml` 固定主版本；升级 `image` / `eframe` 时留意 API 变化，`cargo test` 可覆盖核心行为。

## 10. 原生编解码尝试（结论：未采用）

为提升图片处理速度，曾接入 `libwebp`（`webp` crate）与 `mozjpeg`（`mozjpeg` crate）做对比测试：

| 方案 | 1200×1700 实测 | 结论 |
| --- | --- | --- |
| libwebp 解码 | 33.6 ms | 与纯 Rust 解码（32.6 ms）持平，无收益 |
| libwebp 无损编码 | 278–809 ms | 比纯 Rust（约 15–30 ms）慢一个数量级 |
| mozjpeg 编码 q95 | 约 430 ms | 比纯 Rust（约 46 ms）慢约 9 倍（体积更小） |

因此最终保留纯 Rust 编解码，构建时不需要 C 工具链；同时引入 `mimalloc` 作为分配器获得约 15% 的管线加速。