@echo off
setlocal
chcp 65001 >nul
cd /d "%~dp0"

if exist "JMComic-Downloader-GUI.exe" goto run_root
if exist "target\release\jmcomic-downloader-gui.exe" goto run_target
if exist "dist\JMComic-Downloader-GUI.exe" goto run_dist

echo [ERROR] 未找到可执行文件。
echo 请先构建：.\build_exe.ps1
pause
exit /b 1

:run_root
start "" "JMComic-Downloader-GUI.exe"
exit /b 0

:run_target
start "" "target\release\jmcomic-downloader-gui.exe"
exit /b 0

:run_dist
start "" "dist\JMComic-Downloader-GUI.exe"
exit /b 0
