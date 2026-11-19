#Requires -Version 5.1
<#
.SYNOPSIS
    构建 JMComic Downloader（release），并把 EXE 复制到仓库根目录。
#>
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
if (-not (Test-Path $cargo)) {
    $cargo = "cargo"
}

Write-Host "使用工具链：" -NoNewline
& $cargo --version

& $cargo build --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build 失败（退出码 $LASTEXITCODE）"
}

$source = Join-Path $PSScriptRoot "target\release\jmcomic-downloader-gui.exe"
if (-not (Test-Path $source)) {
    throw "未找到构建产物：$source"
}

$target = Join-Path $PSScriptRoot "JMComic-Downloader-GUI.exe"
Copy-Item $source $target -Force

Write-Host ""
Write-Host "构建完成：$target"
Write-Host "自检：& '$target' --self-test"