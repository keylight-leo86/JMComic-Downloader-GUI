# 安全说明

## 报告安全问题

请不要在公开 Issue 中提交以下内容：

- JM 账号或密码
- Cookie、访问令牌、API Key
- 代理认证信息
- 含个人目录、邮箱或身份信息的完整日志
- 下载内容、收藏夹导出或成人内容截图

请先删除敏感信息，再提供最小化复现步骤。仓库创建后，维护者可在本文件中补充私下联系渠道或启用 GitHub Private Vulnerability Reporting。

## 本地数据

- GUI 设置保存在 `%LOCALAPPDATA%\JMComic-Downloader-GUI\settings.json`，并兼容读取旧版设置。
- 下载内容默认保存在程序同目录的 `下载` 文件夹。
- 浏览器扩展只读取当前窗口标签页的标题和 URL，用于本地筛选与复制，不会主动上传这些信息。
- 项目不会要求用户关闭杀毒软件；Windows SmartScreen 或杀毒软件可能对未签名的 PyInstaller EXE 给出提示，用户应从正式 Release 下载并核对 SHA256。
