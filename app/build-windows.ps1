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
    $baseConfig = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json
    $productName = if ($Qa) { (Get-Content -LiteralPath 'src-tauri/tauri.qa.conf.json' -Raw | ConvertFrom-Json).productName } else { $baseConfig.productName }
    $installerName = '{0}_{1}_x64-setup.exe' -f $productName, $baseConfig.version
    $installer = Get-Item -LiteralPath (Join-Path $PSScriptRoot "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/$installerName")
    if (-not $Qa) {
        $signature = Get-AuthenticodeSignature -LiteralPath $installer.FullName
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $env:MON_WINDOWS_CERTIFICATE_THUMBPRINT -or -not $signature.TimeStamperCertificate) { throw 'Public installer signature verification failed.' }
    }
    $flavor = if ($Qa) { 'qa' } else { 'release' }
    & "$PSScriptRoot/../tools/publish-windows-installer.ps1" -Installer $installer.FullName -Flavor $flavor
} finally { Pop-Location }
