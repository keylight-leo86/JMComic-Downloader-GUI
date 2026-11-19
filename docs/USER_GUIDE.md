# JMComic Downloader 使用说明

## 1. 解压与启动

1. 完整解压 `JMComic-Downloader-GUI-<版本>-Windows-x64.zip`。
2. 双击 `JMComic-Downloader-GUI.exe`。
3. 不需要安装 Python、Java 或任何运行时。

浏览器扩展不能直接从 ZIP 中加载，因此必须先完整解压，并且加载后不要移动 `browser-extension` 文件夹。

## 1.1 界面与主题

- 标题栏右侧的“主题”可选 **跟随系统 / 浅色 / 深色**；默认跟随 Windows 11 的浅色/深色设置，并自动使用系统强调色。
- 标题栏的“缩放”可选 **100% / 110% / 125% / 150%**，默认 100%（相当于旧版本的 125% 观感）；也可用 `Ctrl + =` 放大、`Ctrl + -` 缩小（5% 步进）、`Ctrl + 0` 复位。设置会保存到 `settings.json`，下次启动沿用。
- 左侧为参数卡片，右侧为实时日志（可选中复制），底部为进度条、下载速度、已用时间和操作按钮。
- 输入车号后按 `Ctrl+Enter` 可直接开始下载；`停止` 为协作式取消，正在传输的请求会收尾后退出，已下载图片保留可续传。

## 2. 下载

1. 选择“本子”或“章节”。详情页通常选择“本子”。
2. 在输入框粘贴一个或多个车号/链接。
3. 选择保存位置。
4. 一般保持默认设置。
5. 点击“开始下载”。
6. 完成后点击“打开 PDF 目录”。

多个输入可用空格、逗号、分号或换行分隔。本子与章节请分批处理，因为当前模式会应用于整批输入。

示例：

```text
1455254
JM1455254
https://devapp.18comic.cc/comic/detail?id=1455254
```

## 3. 默认设置

- 保存位置：当前 `JMComic-Downloader-GUI.exe` 所在文件夹下的 `下载`
- 客户端：移动端 API（推荐）
- 图片格式：保持原格式
- 图片并发：32
- 章节并发：8
- 本子并发：4
- 代理：跟随系统
- 高级 YAML：留空

网络不稳定时，可尝试降低并发（例如图片 8、章节 2），或对调移动端 API / 网页端 HTML。

## 4. PDF 与图片

- 不缩小图片分辨率，不主动降低 JPEG 质量。
- JPEG 会以原始字节直接写入 PDF，不重新编码；WebP 等其他格式按解码后的像素无损写入，PDF 可能明显变大。
- PDF 生成后校验页数，校验成功才删除对应中间图片。
- PDF 失败时保留图片，方便排查和重试。
- 单章节本子生成 `本子名.pdf`；多章节本子生成 `本子名 - 第N话 章节名.pdf`。
- 正常完成后最终文件位于保存位置下的 `PDF` 文件夹。

## 5. 停止任务

点击“停止”后不会立即中断正在传输的数据，而是不再发起新请求，正在进行的请求收尾后退出；已下载的图片会保留，下次运行会跳过已存在的图片。

## 6. 命令行（可选）

```powershell
# 下载整本
.\JMComic-Downloader-GUI.exe --cli --kind album --id 1455254 --output D:\Comics

# 自检（不联网）
.\JMComic-Downloader-GUI.exe --self-test

# 查看全部参数
.\JMComic-Downloader-GUI.exe --help
```

## 7. 高级 YAML 配置

支持 Python 版配置中与下载相关的子集：

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

`plugins` 等未支持的配置会在日志中提示后忽略。

## 8. 常见问题

### Windows 显示未知发布者

当前 EXE 未进行商业代码签名。请从项目正式 GitHub Release 下载，并使用 `SHA256SUMS.txt` 核对文件；不要关闭杀毒软件。

### 一直重试 / 无法连接

1. 检查代理设置（默认“跟随系统”）；
2. 尝试在移动端 API 与网页端 HTML 之间切换；
3. 域名可能已更换，可用命令行参数 `--api-domain www.cdngwc.cc` 指定；
4. 查看右侧日志中的具体错误（HTTP 状态码、域名、重试次数）。

### 扩展没有识别到页面

确认页面与扩展位于同一个浏览器窗口，扩展版本为 1.0.2，并在扩展管理页点击“重新加载”。

### 设置存放位置

```text
%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json
```

删除该设置文件可恢复默认值，不会删除已下载 PDF。

默认保存位置会随 EXE 所在文件夹变化；只有手动选择的自定义保存位置才会跨启动保留。

## 9. 使用边界

本项目不包含任何漫画内容。请仅下载自己有权访问和保存的内容，并遵守法律、网站条款和版权要求。