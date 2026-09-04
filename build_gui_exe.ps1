param(
    [string]$PythonPath = (Join-Path $PSScriptRoot '.venv\Scripts\python.exe'),
    [switch]$IncludeWorker
)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

# ---------------------------------------------------------------------------
# 新版桌面界面（run_gui.py，pywebview + WebView2 双窗口壳 + 在线预览 + 下载任务）EXE 构建。
#
# 默认（单 exe）：只产出一个主程序 onefile exe，分发时一个文件即可运行：
#   JMComic-Downloader-GUI.exe   主程序（--windowed，pywebview 桌面窗口，WebView2 渲染）
#                                —— 壳内预创建两个窗口：主窗 + 独立阅读窗（frameless +
#                                WndProc 子类化自绘标题栏；阅读窗常驻隐藏、关闭=隐藏复用）。
#                                —— 内嵌下载 worker 入口：run_gui.py 识别首参
#                                `--jm-worker` 即转交 gui.worker.main()（发生在加载
#                                pywebview/服务之前），jobs.py 打包模式发现同目录无
#                                Worker.exe 时以「本 exe --jm-worker」自我拉起下载子进程。
#
# 可选双 exe（加 -IncludeWorker，旧布局，下载子进程启动更快、内存占用更小）：
#   JMComic-Downloader-GUI.exe      主程序
#   JMComic-Downloader-Worker.exe   下载子进程（--console，主程序优先用它、隐藏拉起）
#
# 说明：现有 build_exe.ps1 打包的是旧 tkinter 界面（jmcomic_gui.py），二者互不影响。
# WebView2：走系统 WebView2 Runtime（Edge 自带），不打包 Chromium。运行期经
# webview.start(storage_path=...) 指定专属 UserDataFolder（exe 同目录 .webview2-cache），
# 与系统其它 WebView2 实例（浏览器/其它应用）数据隔离，互不影响。
# 收集项：webview 的 .NET 程序集/WebView2Loader.dll、clr_loader、pythonnet
# （Python.Runtime.dll）是 winforms 后端跑通 pythonnet 桥接所必需。
# ---------------------------------------------------------------------------

if (-not (Test-Path -LiteralPath $PythonPath)) {
    python -m venv .venv
    $PythonPath = Join-Path $PSScriptRoot '.venv\Scripts\python.exe'
}

# jmcomic 是内嵌于 src/ 的源码包：collect_all('jmcomic')/modulegraph 依赖 sys.path
# 能 import 到它。脚本虽先 `pip install -e .`（editable 会把 src 挂进 sys.path），
# 但该步骤一旦失败/包结构变动就会静默漏收（PyInstaller 对 collect 不到的包不报错，
# exe 内 jmcomic=0 → 运行期一搜索就 No module named）。--paths 显式补 src 兜底。
$srcPath = Join-Path $PSScriptRoot 'src'
$pathsArg = @('--paths', $srcPath)

& $pythonPath -m pip install --upgrade pip
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 运行依赖（含 pywebview/pythonnet）+ 构建器
& $pythonPath -m pip install -r (Join-Path $PSScriptRoot 'requirements.txt') PyInstaller==6.21.0
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& $pythonPath -m pip install -e . --no-deps
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$distDir = Join-Path $PSScriptRoot 'dist\gui'
$buildDir = Join-Path $PSScriptRoot 'build\gui'
New-Item -ItemType Directory -Force -Path $distDir, $buildDir | Out-Null

# 下载/PDF 相关第三方包（与旧 build_exe.ps1 保持一致）
$dlCollect = @(
    '--collect-all', 'jmcomic',
    '--collect-all', 'common',
    '--collect-all', 'curl_cffi',
    '--collect-all', 'pypdf',
    '--collect-all', 'requests',
    '--exclude-module', 'pikepdf',
    '--exclude-module', 'lxml'
)

# pywebview 桌面壳运行时：webview(.NET 程序集/WebView2Loader) + pythonnet 桥接链
$webviewCollect = @(
    '--collect-all', 'webview',
    '--collect-all', 'clr_loader',
    '--collect-all', 'pythonnet'
)

$workPath = Join-Path $buildDir 'work'
$specPath = Join-Path $buildDir 'spec'

# ---- 主程序：windowed + pywebview(WebView2) + static 前端（单 exe 分发，内嵌 worker）----
# --hidden-import gui.worker：run_gui.py 对 --jm-worker 的分派 import 发生在运行时
# 条件分支，需显式声明，确保 worker 及其依赖（jmcomic_gui 等）随主包收集。
$mainArgs = @(
    '--noconfirm', '--clean',
    '--onefile', '--windowed',
    '--name', 'JMComic-Downloader-GUI',
    '--distpath', $distDir,
    '--workpath', $workPath,
    '--specpath', $specPath,
    '--hidden-import', 'gui.worker'
) + $pathsArg + $dlCollect + $webviewCollect
$mainArgs += @(
    '--add-data', "$(Join-Path $PSScriptRoot 'static');static"
)
$mainArgs += (Join-Path $PSScriptRoot 'run_gui.py')
& $pythonPath -m PyInstaller @mainArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host ''
Write-Host "[build_gui_exe] 主程序 done ->"
Write-Host "  $(Join-Path $distDir 'JMComic-Downloader-GUI.exe')"

if ($IncludeWorker) {
    # ---- 可选下载 worker：console；import jmcomic_gui → tkinter ----
    # tk/tcl 由 PyInstaller 内置 tkinter hook 自动收集（DLL 位于 base 的 DLLs\ 或 Library\bin，
    # 视 Python 版本而定），无需手工 add-binary。旧版 build_exe.ps1 的手工 DLL 列表面向
    # 旧 Python 发行布局（Library\bin），在 3.13+ 已静态链接进扩展模块，这里不再需要。
    $workerArgs = @(
        '--noconfirm', '--clean',
        '--onefile', '--console',
        '--name', 'JMComic-Downloader-Worker',
        '--distpath', $distDir,
        '--workpath', $workPath,
        '--specpath', $specPath
    ) + $pathsArg + $dlCollect
    $workerArgs += (Join-Path $PSScriptRoot 'gui\worker.py')
    & $pythonPath -m PyInstaller @workerArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host ''
    Write-Host "[build_gui_exe] Worker done ->"
    Write-Host "  $(Join-Path $distDir 'JMComic-Downloader-Worker.exe')"
    Write-Host '双 exe 布局：两者须保持同目录，主程序优先以隐藏窗口拉起 Worker 子进程。'
} else {
    Write-Host '单 exe 布局：下载任务由主程序以 `--jm-worker` 自我拉起，一个文件即可分发。'
    Write-Host '（如需旧双 exe 布局，重新执行时加 -IncludeWorker。）'
}
exit 0
