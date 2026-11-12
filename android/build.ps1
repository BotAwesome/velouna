# Builds the Velour APK.
#
#   pwsh android\build.ps1                 # debug APK, arm64 + x86_64
#   pwsh android\build.ps1 -Abi arm64-v8a  # phones only (faster)
#   pwsh android\build.ps1 -Run            # also install on the adb device and start
#   pwsh android\build.ps1 -Release        # optimized Rust + release APK
param(
    [string[]]$Abi = @("arm64-v8a", "x86_64"),
    [switch]$Release,
    [switch]$Run
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$root = Split-Path $here -Parent

if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk" }
if (-not $env:ANDROID_NDK_HOME) {
    $ndk = Get-ChildItem "$env:ANDROID_HOME\ndk" -Directory | Sort-Object Name -Descending | Select-Object -First 1
    if (-not $ndk) { throw "Android NDK not found under $env:ANDROID_HOME\ndk" }
    $env:ANDROID_NDK_HOME = $ndk.FullName
}
if (-not (Test-Path "$here\local.properties")) {
    "sdk.dir=$($env:ANDROID_HOME -replace '\\', '/')" | Set-Content "$here\local.properties"
}

# Rust library -> app/src/main/jniLibs/<abi>/libvelour_android.so
$jni = "$here\app\src\main\jniLibs"
$targets = @(); foreach ($a in $Abi) { $targets += "-t"; $targets += $a }
# (Not $profile: that is PowerShell's own variable, and a one-element array
# from `if` collapses to a string, which splatting would split into chars.)
$cargoArgs = @("ndk") + $targets + @("-P", "26", "-o", $jni, "build", "-p", "velour-android")
if ($Release) { $cargoArgs += "--release" }
Push-Location $root
try {
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed" }
} finally { Pop-Location }

# APK (version from Cargo.toml, so in-app updates compare the same numbers)
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$task = if ($Release) { "assembleRelease" } else { "assembleDebug" }
Push-Location $here
try {
    & .\gradlew.bat $task "-Pabis=$($Abi -join ',')" "-PvelourVersion=$version" --console=plain -q
    if ($LASTEXITCODE -ne 0) { throw "gradle $task failed" }
} finally { Pop-Location }
$variant = if ($Release) { "release" } else { "debug" }
$apk = Get-ChildItem "$here\app\build\outputs\apk\$variant\*.apk" | Select-Object -First 1
Write-Host "APK: $($apk.FullName) ($([math]::Round($apk.Length / 1MB, 1)) MB)" -ForegroundColor Cyan

if ($Run) {
    adb install -r $apk.FullName
    if ($LASTEXITCODE -ne 0) { throw "adb install failed" }
    adb shell am start -n app.velour/.VelourActivity
}
