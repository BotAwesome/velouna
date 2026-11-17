# Builds a release and publishes it on GitHub.
#
#   pwsh tools\release.ps1             # build + check only (dry run)
#   pwsh tools\release.ps1 -Publish    # also tag, upload and publish
#   pwsh tools\release.ps1 -Publish -Draft
#
# Version: Cargo.toml. Release text: the matching "## [x.y.z]" section of
# CHANGELOG.md. Assets: dist\Velour-Setup-<v>.exe, dist\Velour-<v>-arm64.apk,
# dist\velour_<v>_amd64.deb and dist\velour-<v>-linux-x86_64.tar.gz; the app's
# updater picks them by their endings (.exe with "Setup", .apk, _amd64.deb,
# -linux-x86_64.tar.gz).
param(
    [switch]$Publish,
    [switch]$Draft,
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tag = "v$version"
Write-Host "Velour $version" -ForegroundColor Cyan

# Release notes from the changelog.
$notes = @(); $take = $false
foreach ($line in Get-Content CHANGELOG.md) {
    if ($line -match '^## ') {
        if ($take) { break }
        $take = $line -match "^## \[?$([regex]::Escape($version))\]?"
        continue
    }
    if ($take) { $notes += $line }
}
$notesText = ($notes -join "`n").Trim()
if (-not $notesText) { throw "CHANGELOG.md has no section for $version" }
$notesFile = Join-Path $root "dist\release-notes-$version.md"
New-Item -ItemType Directory -Force (Join-Path $root "dist") | Out-Null
Set-Content -Encoding utf8NoBOM $notesFile $notesText

if ($Publish) {
    if (git status --porcelain) { throw "Uncommitted changes: commit them first." }
    git fetch --tags origin 2>$null
    if (git tag -l $tag) { throw "Tag $tag exists already: raise the version in Cargo.toml." }
}

$setup = Join-Path $root "dist\Velour-Setup-$version.exe"
$apk = Join-Path $root "dist\Velour-$version-arm64.apk"
$deb = Join-Path $root "dist\velour_${version}_amd64.deb"
$tgz = Join-Path $root "dist\velour-$version-linux-x86_64.tar.gz"
if (-not $SkipBuild) {
    pwsh (Join-Path $root "installer\build.ps1")
    if ($LASTEXITCODE -ne 0) { throw "installer build failed" }
    pwsh (Join-Path $root "android\build.ps1") -Abi arm64-v8a -Release
    if ($LASTEXITCODE -ne 0) { throw "APK build failed" }
    Copy-Item (Join-Path $root "android\app\build\outputs\apk\release\app-release.apk") $apk -Force
    # Linux: cross-compiled with cargo-zigbuild (zig from `pip install ziglang`);
    # the bundled mpv/FFmpeg runtime is dist\linux-runtime, built on the VPS by
    # tools/linux/build-libs.sh.
    $zig = python -c "import os, ziglang; print(os.path.dirname(ziglang.__file__))"
    if ($LASTEXITCODE -ne 0) { throw "zig missing: pip install ziglang" }
    $env:PATH = "$zig;$env:PATH"
    cargo zigbuild --release -p velour --target x86_64-unknown-linux-gnu.2.31
    if ($LASTEXITCODE -ne 0) { throw "Linux build failed" }
    Remove-Item Env:VELOUR_BETA -ErrorAction SilentlyContinue
    python (Join-Path $root "tools\linux\package.py")
    if ($LASTEXITCODE -ne 0) { throw "Linux packaging failed" }
}
foreach ($f in $setup, $apk, $deb, $tgz) {
    if (-not (Test-Path $f)) { throw "missing $f" }
}
# The APK must carry the release key: a debug-signed one can't update
# installed copies (see android/app/build.gradle.kts).
$bt = Get-ChildItem "$(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "$env:LOCALAPPDATA\Android\Sdk" })\build-tools" -Directory | Sort-Object { [version]($_.Name -replace '-.*') } | Select-Object -Last 1
$cert = & (Join-Path $bt.FullName "apksigner.bat") verify --print-certs $apk 2>$null | Out-String
if ($LASTEXITCODE -ne 0) { throw "apksigner could not verify $apk" }
if ($cert -match "CN=Android Debug") { throw "The APK is signed with the debug key: put the release key into ~/.velour/signing (or set VELOUR_SIGNING)." }
$certSha = [regex]::Match($cert, "SHA-256 digest: ([0-9a-f]+)").Groups[1].Value
Write-Host "APK signer: $([regex]::Match($cert, 'DN: (.+)').Groups[1].Value.Trim())  sha256 $certSha"
foreach ($f in $setup, $apk, $deb, $tgz) {
    "{0}  {1:N1} MB  sha256 {2}" -f (Split-Path $f -Leaf), ((Get-Item $f).Length / 1MB), (Get-FileHash $f -Algorithm SHA256).Hash.ToLower()
}
Write-Host "`nRelease notes:" -ForegroundColor Cyan
$notesText

if (-not $Publish) {
    Write-Host "`nDry run. Publish with: pwsh tools\release.ps1 -Publish -SkipBuild" -ForegroundColor Yellow
    exit 0
}
git push origin HEAD
if ($LASTEXITCODE -ne 0) { throw "git push failed" }
$ghArgs = @("release", "create", $tag, $setup, $apk, $deb, $tgz, "--title", "Velour $version", "--notes-file", $notesFile, "--target", (git rev-parse HEAD))
if ($Draft) { $ghArgs += "--draft" }
gh @ghArgs
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
Write-Host "Published $tag" -ForegroundColor Green
