param([switch]$Qa)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build this installer on Windows.' }
Push-Location $PSScriptRoot
try {
    $env:PYTHONUTF8 = '1'
    $env:MON_WINDOWS_RELEASE_SIGNING = if ($Qa) { '0' } else { '1' }
    if (-not $Qa) {
        & "$PSScriptRoot/../tools/sign-windows.ps1" -ValidateOnly
        if (-not $?) { throw 'Public release signing prerequisites failed.' }
        $signing = @{ bundle = @{ windows = @{ signCommand = @{
            cmd = 'powershell.exe'
            args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                (Join-Path $PSScriptRoot '../tools/sign-windows.ps1'), '-Path', '%1')
        } } } } | ConvertTo-Json -Depth 8 -Compress
    }
    $tauriArgs = @('run', 'tauri', '--', 'build', '--target', 'x86_64-pc-windows-msvc',
        '--config', 'src-tauri/tauri.windows.bundle.conf.json')
    if ($Qa) { $tauriArgs += @('--config', 'src-tauri/tauri.qa.conf.json') }
    else { $tauriArgs += @('--config', $signing) }
    $tauriArgs += @('--', '--locked')
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) { throw "Tauri build failed ($LASTEXITCODE)." }
    if (-not $Qa) {
        $installers = @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis') -Filter '*.exe' -File)
        if ($installers.Count -eq 0) { throw 'No public installer was produced.' }
        foreach ($installer in $installers) {
            $signature = Get-AuthenticodeSignature -LiteralPath $installer.FullName
            if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $env:MON_WINDOWS_CERTIFICATE_THUMBPRINT -or -not $signature.TimeStamperCertificate) { throw 'Public installer signature verification failed.' }
        }
    }
} finally { Pop-Location }
