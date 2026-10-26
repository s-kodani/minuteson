$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$env:PYTHONUTF8 = '1'
if ($env:OS -ne 'Windows_NT') { throw 'Prepare Windows resources on Windows.' }
Push-Location $PSScriptRoot
try {
    & python ../tools/mon_windows_capture_dependencies.py
    if ($LASTEXITCODE -ne 0) { throw 'Capture protocol dependency preparation failed.' }
    & python ../tools/mon_windows_runtime.py
    if ($LASTEXITCODE -ne 0) { throw 'Windows inference runtime preparation failed.' }
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
    & python ../tools/mon_third_party_notices.py --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Third-party notice generation failed.' }
    & cmake -S ../probes/windows-capture -B ../probes/windows-capture/build -G 'Visual Studio 17 2022' -A x64 -DBUILD_TESTING=ON
    if ($LASTEXITCODE -ne 0) { throw 'Capture probe configuration failed.' }
    & cmake --build ../probes/windows-capture/build --config Release
    if ($LASTEXITCODE -ne 0) { throw 'Capture probe build failed.' }
    $binaries = Join-Path $PSScriptRoot 'src-tauri/binaries'
    New-Item -ItemType Directory -Force -Path $binaries | Out-Null
    Copy-Item '../probes/windows-capture/build/Release/mon-windows-capture.exe' `
        (Join-Path $binaries 'mon-windows-capture-x86_64-pc-windows-msvc.exe') -Force
    if ($env:MON_WINDOWS_RELEASE_SIGNING -eq '1') {
        & "$PSScriptRoot/../tools/sign-windows.ps1" -Path (Join-Path $binaries 'mon-windows-capture-x86_64-pc-windows-msvc.exe')
        if (-not $?) { throw 'Capture sidecar signing failed.' }
    }
} finally { Pop-Location }
