$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

$pythonPath = Join-Path $PSScriptRoot '.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $pythonPath)) {
    python -m venv .venv
}

& $pythonPath -m pip install --upgrade pip
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& $pythonPath -m pip install -r (Join-Path $PSScriptRoot 'requirements-release.txt')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& $pythonPath -m pip install -e . --no-deps
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exeName = 'JMComic-Downloader-GUI'
$basePrefix = & $pythonPath -c 'import sys; print(sys.base_prefix)'
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$runtimeBin = Join-Path $basePrefix 'Library\bin'
$requiredDlls = @('liblzma.dll', 'libbz2.dll', 'ffi.dll', 'libexpat.dll', 'tk86t.dll', 'tcl86t.dll')
$pyInstallerArgs = @(
    '--noconfirm',
    '--clean',
    '--onefile',
    '--windowed',
    '--name', $exeName,
    '--distpath', $PSScriptRoot,
    '--workpath', (Join-Path $PSScriptRoot 'build'),
    '--specpath', (Join-Path $PSScriptRoot 'build'),
    '--collect-all', 'jmcomic',
    '--collect-all', 'common',
    '--collect-all', 'curl_cffi',
    '--collect-all', 'pypdf',
    '--collect-all', 'requests',
    '--exclude-module', 'pikepdf',
    '--exclude-module', 'lxml'
)

foreach ($dllName in $requiredDlls) {
    $dllPath = Join-Path $runtimeBin $dllName
    if (-not (Test-Path -LiteralPath $dllPath)) {
        throw "Required runtime DLL not found: $dllPath"
    }
    $pyInstallerArgs += @('--add-binary', "$dllPath;.")
}

$pyInstallerArgs += (Join-Path $PSScriptRoot 'jmcomic_gui.py')
& $pythonPath -m PyInstaller @pyInstallerArgs

exit $LASTEXITCODE
