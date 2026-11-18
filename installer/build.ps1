# Builds dist\Velouna-Setup-<version>.exe (the custom installer in setup/).
#
#   pwsh installer\build.ps1 [-FFmpegDir <folder with ffmpeg.exe/ffprobe.exe>]
#
# Bundled: velouna.exe, lib\libmpv-2.dll, ffmpeg.exe, ffprobe.exe and license
# texts. FFmpeg comes from -FFmpegDir or from PATH.
param(
    [string]$FFmpegDir = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$stage = Join-Path $PSScriptRoot "stage"

if (-not $FFmpegDir) {
    $cmd = Get-Command ffmpeg.exe -ErrorAction SilentlyContinue
    if (-not $cmd) { throw "ffmpeg.exe not found on PATH; pass -FFmpegDir" }
    $FFmpegDir = Split-Path $cmd.Source -Parent
}
foreach ($t in "ffmpeg.exe", "ffprobe.exe") {
    if (-not (Test-Path (Join-Path $FFmpegDir $t))) { throw "$t missing in $FFmpegDir" }
}
$mpv = Join-Path $root "lib\libmpv-2.dll"
if (-not (Test-Path $mpv)) { throw "lib\libmpv-2.dll missing" }

$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Velouna $version" -ForegroundColor Cyan

Push-Location $root
try {
    cargo build --release -p velouna -p velouna-setup
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

# License texts
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\FFmpeg" | Out-Null
$ffRoot = Split-Path $FFmpegDir -Parent
foreach ($f in "LICENSE", "README.txt") {
    $p = Join-Path $ffRoot $f
    if (Test-Path $p) { Copy-Item $p "$stage\FFmpeg\" }
}
@"
Velouna bundles the following third-party components:

mpv / libmpv (libmpv-2.dll)
  License: GPL-2.0-or-later / LGPL-2.1-or-later (depending on build)
  Source:  https://github.com/mpv-player/mpv

FFmpeg (ffmpeg.exe, ffprobe.exe)
  License and build details: licenses\FFmpeg\
  Source:  https://github.com/FFmpeg/FFmpeg

egui / eframe (MIT OR Apache-2.0), Phosphor Icons (MIT)
"@ | Set-Content -Encoding utf8 "$stage\THIRD-PARTY.txt"

$entries = @(
    "velouna.exe=$root\target\release\velouna.exe",
    "libmpv-2.dll=$mpv",
    "ffmpeg.exe=$FFmpegDir\ffmpeg.exe",
    "ffprobe.exe=$FFmpegDir\ffprobe.exe",
    "licenses/THIRD-PARTY.txt=$stage\THIRD-PARTY.txt"
)
Get-ChildItem "$stage\FFmpeg" -File | ForEach-Object { $entries += "licenses/FFmpeg/$($_.Name)=$($_.FullName)" }

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force $dist | Out-Null
$out = Join-Path $dist "Velouna-Setup-$version.exe"
& "$root\target\release\velouna-pack.exe" "$root\target\release\velouna-setup.exe" $out $version @entries
if ($LASTEXITCODE -ne 0) { throw "packing failed" }
