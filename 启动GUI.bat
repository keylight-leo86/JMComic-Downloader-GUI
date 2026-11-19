@echo off
chcp 65001 >nul
cd /d "%~dp0"

if exist "JMComic-Downloader-GUI.exe" (
  start "" "JMComic-Downloader-GUI.exe"
  exit /b 0
)
if exist "target\release\jmcomic-downloader-gui.exe" (
  start "" "target\release\jmcomic-downloader-gui.exe"
  exit /b 0
)

echo [ERROR] 未找到可执行文件。
echo 请先构建：.\build_exe.ps1
pause
exit /b 1