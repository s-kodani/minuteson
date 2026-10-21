param([switch]$Qa)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build this installer on Windows.' }
Push-Location $PSScriptRoot
try {
    $tauriArgs = @('run', 'tauri', '--', 'build', '--target', 'x86_64-pc-windows-msvc',
        '--config', 'src-tauri/tauri.windows.bundle.conf.json')
    if ($Qa) { $tauriArgs += @('--config', 'src-tauri/tauri.qa.conf.json') }
    $tauriArgs += @('--', '--locked')
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) { throw "Tauri build failed ($LASTEXITCODE)." }
} finally { Pop-Location }
