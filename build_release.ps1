$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

$version = '1.2.0'
$assetBase = "JMComic-Downloader-GUI-$version-Windows-x64"
$releaseRoot = Join-Path $PSScriptRoot 'release-assets'
$stagingRoot = Join-Path $releaseRoot $assetBase
$zipPath = Join-Path $releaseRoot "$assetBase.zip"
$zipHashPath = "$zipPath.sha256.txt"
$exePath = Join-Path $PSScriptRoot 'JMComic-Downloader-GUI.exe'
$downloadDirName = -join @([char]0x4E0B, [char]0x8F7D)
$userGuideName = (-join @([char]0x4F7F, [char]0x7528, [char]0x8BF4, [char]0x660E)) + '.md'

if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Missing EXE: $exePath. Run .\build_exe.ps1 first."
}

if (Test-Path -LiteralPath $stagingRoot) {
    throw "Release staging already exists: $stagingRoot"
}
if (Test-Path -LiteralPath $zipPath) {
    throw "Release ZIP already exists: $zipPath"
}

New-Item -ItemType Directory -Path $stagingRoot | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stagingRoot 'browser-extension') | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stagingRoot 'licenses') | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stagingRoot (Join-Path $downloadDirName 'PDF')) | Out-Null

Copy-Item -LiteralPath $exePath -Destination (Join-Path $stagingRoot 'JMComic-Downloader-GUI.exe')
foreach ($name in @('README.md', 'LICENSE', 'NOTICE', 'UPSTREAM.md', 'THIRD_PARTY_NOTICES.md', 'DISCLAIMER.md')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $stagingRoot $name)
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'docs\USER_GUIDE.md') -Destination (Join-Path $stagingRoot $userGuideName)
$licenseSource = Join-Path $PSScriptRoot 'third_party_licenses'
if (-not (Test-Path -LiteralPath $licenseSource)) {
    throw "Missing license directory: $licenseSource"
}
foreach ($file in Get-ChildItem -LiteralPath $licenseSource -File) {
    Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $stagingRoot 'licenses')
}

$extensionFiles = @('manifest.json', 'popup.html', 'popup.css', 'popup.js', 'extractor.js', 'README.md')
foreach ($name in $extensionFiles) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "browser-extension\$name") -Destination (Join-Path $stagingRoot "browser-extension\$name")
}

$checksums = @()
foreach ($file in Get-ChildItem -LiteralPath $stagingRoot -Recurse -File | Sort-Object FullName) {
    $relative = $file.FullName.Substring($stagingRoot.Length + 1).Replace('\', '/')
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksums += "$hash  $relative"
}
Set-Content -LiteralPath (Join-Path $stagingRoot 'SHA256SUMS.txt') -Value $checksums -Encoding UTF8

Compress-Archive -LiteralPath $stagingRoot -DestinationPath $zipPath -CompressionLevel Optimal
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath $zipHashPath -Value "$zipHash  $assetBase.zip" -Encoding UTF8

Write-Output "Release asset: $zipPath"
Write-Output "SHA256 file: $zipHashPath"
