# Builds a release and publishes it on GitHub.
#
#   pwsh tools\release.ps1             # build + check only (dry run)
#   pwsh tools\release.ps1 -Publish    # also tag, upload and publish
#   pwsh tools\release.ps1 -Publish -Draft
#
# Version: Cargo.toml. Release text: the matching "## [x.y.z]" section of
# CHANGELOG.md. Assets: dist\Velour-Setup-<v>.exe and dist\Velour-<v>-arm64.apk;
# the app's updater picks them by their endings (.exe with "Setup", .apk).
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
if (-not $SkipBuild) {
    pwsh (Join-Path $root "installer\build.ps1")
    if ($LASTEXITCODE -ne 0) { throw "installer build failed" }
    pwsh (Join-Path $root "android\build.ps1") -Abi arm64-v8a -Release
    if ($LASTEXITCODE -ne 0) { throw "APK build failed" }
    Copy-Item (Join-Path $root "android\app\build\outputs\apk\release\app-release.apk") $apk -Force
}
foreach ($f in $setup, $apk) {
    if (-not (Test-Path $f)) { throw "missing $f" }
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
$ghArgs = @("release", "create", $tag, $setup, $apk, "--title", "Velour $version", "--notes-file", $notesFile, "--target", (git rev-parse HEAD))
if ($Draft) { $ghArgs += "--draft" }
gh @ghArgs
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
Write-Host "Published $tag" -ForegroundColor Green
