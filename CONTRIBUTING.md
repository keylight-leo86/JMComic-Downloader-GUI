# 贡献指南

欢迎提交问题和改进。提交前请注意：

1. 不要提交下载目录、PDF、漫画图片、账号、Cookie、Token 或真实个人路径。
2. GUI/PDF 改动至少运行：

   ```powershell
   python -m unittest tests.test_jmcomic_gui_pdf
   ```

3. 浏览器扩展改动至少运行：

   ```powershell
   cd browser-extension
   npm test
   ```

4. 修改公开功能时同步更新 `README.md`。
5. 修改第三方依赖时同步检查 `THIRD_PARTY_NOTICES.md`。

本项目是上游 `JMComic-Crawler-Python` 的非官方衍生版本。上游通用问题应先确认是否能在原项目复现；本项目的 GUI、PDF 和浏览器扩展问题可在本仓库报告。
