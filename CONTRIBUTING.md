# 贡献指南

欢迎提交问题和改进。提交前请注意：

1. 不要提交下载目录、PDF、漫画图片、账号、Cookie、Token 或真实个人路径。
2. 下载核心 / GUI / PDF 改动至少运行：

   ```powershell
   cargo fmt --check
   cargo test
   cargo run --release -- --self-test
   ```

3. 浏览器扩展改动至少运行：

   ```powershell
   cd browser-extension
   npm test
   ```

4. 修改公开功能时同步更新 `README.md`。
5. 修改第三方依赖时同步检查 `THIRD_PARTY_NOTICES.md`。

本项目是上游 `JMComic-Crawler-Python` 的非官方衍生版本（Rust 重写）。上游通用问题应先确认是否能在原项目复现；本项目的下载链路、GUI、PDF 和浏览器扩展问题可在本仓库报告。
