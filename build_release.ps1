#Requires -Version 5.1
<#
.SYNOPSIS
    构建 Rust 版发布包：编译 release、运行离线自检、整理 dist 目录、生成 ZIP 与 SHA256。

.DESCRIPTION
    产物（rust\dist）：
      JMComic-Downloader-GUI.exe
      使用说明.md / README.md / LICENSE / NOTICE ...
      browser-extension\
      SHA256SUMS.txt
      JMComic-Downloader-GUI-<版本>-Windows-x64.zip
#>
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$repoRoot = $PSScriptRoot
$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
if (-not (Test-Path $cargo)) {
    $cargo = "cargo"
}

Write-Host "==> 运行测试"
& $cargo test --release --quiet
if ($LASTEXITCODE -ne 0) {
    throw "cargo test 失败（退出码 $LASTEXITCODE）"
}

Write-Host "==> 编译 release"
& $cargo build --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build 失败（退出码 $LASTEXITCODE）"
}

$exeSource = Join-Path $PSScriptRoot "target\release\jmcomic-downloader-gui.exe"
if (-not (Test-Path $exeSource)) {
    throw "未找到构建产物：$exeSource"
}

$versionLine = (& $exeSource --version) -join ""
$version = ($versionLine -split '\s+')[-1]
if (-not $version) {
    throw "无法解析版本号：$versionLine"
}
Write-Host "==> 版本：$version"

Write-Host "==> 离线自检"
& $exeSource --self-test
if ($LASTEXITCODE -ne 0) {
    throw "自检失败（退出码 $LASTEXITCODE）"
}

$dist = Join-Path $PSScriptRoot "dist"
if (Test-Path $dist) {
    Remove-Item $dist -Recurse -Force
}
New-Item -ItemType Directory -Path $dist | Out-Null

$exeTarget = Join-Path $dist "JMComic-Downloader-GUI.exe"
Copy-Item $exeSource $exeTarget -Force

# 文档与许可证
$docFiles = @(
    @{ Source = Join-Path $repoRoot "docs\USER_GUIDE.md"; Target = "使用说明.md" },
    @{ Source = Join-Path $repoRoot "README.md";          Target = "README.md" },
    @{ Source = Join-Path $repoRoot "docs\RUST_MIGRATION.md"; Target = "迁移说明.md" },
    @{ Source = Join-Path $repoRoot "LICENSE";            Target = "LICENSE" },
    @{ Source = Join-Path $repoRoot "NOTICE";             Target = "NOTICE" },
    @{ Source = Join-Path $repoRoot "DISCLAIMER.md";      Target = "DISCLAIMER.md" },
    @{ Source = Join-Path $repoRoot "SECURITY.md";        Target = "SECURITY.md" },
    @{ Source = Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"; Target = "THIRD_PARTY_NOTICES.md" }
)
foreach ($doc in $docFiles) {
    if (Test-Path $doc.Source) {
        Copy-Item $doc.Source (Join-Path $dist $doc.Target) -Force
    }
}

# 浏览器扩展
$extensionSource = Join-Path $repoRoot "browser-extension"
if (Test-Path $extensionSource) {
    Copy-Item $extensionSource (Join-Path $dist "browser-extension") -Recurse -Force
}

# 校验和
$hashLine = Get-FileHash -Algorithm SHA256 -Path $exeTarget
"$($hashLine.Hash.ToLower())  JMComic-Downloader-GUI.exe" |
    Set-Content -Path (Join-Path $dist "SHA256SUMS.txt") -Encoding utf8

# ZIP
$zipName = "JMComic-Downloader-GUI-$version-Windows-x64.zip"
$zipPath = Join-Path $dist $zipName
Compress-Archive -Path (Join-Path $dist "*") -DestinationPath $zipPath -Force
$zipHash = Get-FileHash -Algorithm SHA256 -Path $zipPath
Add-Content -Path (Join-Path $dist "SHA256SUMS.txt") -Value "$($zipHash.Hash.ToLower())  $zipName" -Encoding utf8

Write-Host ""
Write-Host "发布包已生成："
Get-ChildItem $dist | Select-Object Name, Length | Format-Table -AutoSize
Write-Host "校验和：$dist\SHA256SUMS.txt"