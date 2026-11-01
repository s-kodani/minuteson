param([switch]$Qa, [switch]$Unsigned)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Qa -and $Unsigned) { throw '-Qa and -Unsigned cannot be combined.' }
if ($env:OS -ne 'Windows_NT') { throw 'Build this installer on Windows.' }
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\', '/')
function Assert-WorkspaceDirectory([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Build target must stay inside the workspace: $full"
    }
    $cursor = $full
    while ($cursor.Length -gt $workspace.Length) {
        if (Test-Path -LiteralPath $cursor) {
            $entry = Get-Item -LiteralPath $cursor -Force
            if (-not $entry.PSIsContainer -or ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "Build target must use ordinary workspace directories: $cursor"
            }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    return $full
}
$savedEnvironment = @{}
foreach ($name in @('PYTHONUTF8', 'MON_WINDOWS_RELEASE_SIGNING', 'TAURI_CONFIG', 'CARGO_TARGET_DIR', 'PATH')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location $PSScriptRoot
try {
    $env:PYTHONUTF8 = '1'
    $signedRelease = -not ($Qa -or $Unsigned)
    $env:MON_WINDOWS_RELEASE_SIGNING = if ($signedRelease) { '1' } else { '0' }
    if ($signedRelease) {
        & "$PSScriptRoot/../tools/sign-windows.ps1" -ValidateOnly
        if (-not $?) { throw 'Public release signing prerequisites failed.' }
        $signing = @{ bundle = @{ windows = @{ signCommand = @{
            cmd = 'powershell.exe'
            args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                (Join-Path $PSScriptRoot '../tools/sign-windows.ps1'), '-Path', '%1')
        } } } } | ConvertTo-Json -Depth 8 -Compress
    }
    $targetDirectory = Join-Path $PSScriptRoot 'src-tauri/target'
    if ($Unsigned) {
        # A separately compiled app cannot reuse a previously signed Cargo
        # output. Ignore any ambient Tauri signing/configuration overlay.
        $targetDirectory = Assert-WorkspaceDirectory (Join-Path $PSScriptRoot 'src-tauri/target-unsigned')
        $env:CARGO_TARGET_DIR = $targetDirectory
        Remove-Item Env:TAURI_CONFIG -ErrorAction SilentlyContinue
        $signing = @{ bundle = @{ windows = @{ signCommand = $null; certificateThumbprint = $null } } } |
            ConvertTo-Json -Depth 8 -Compress
    }
    # The resource preparation hook also invokes cmake. VS2022 installs it
    # outside PATH on some machines; resolve the actual ctest executable too.
    $cmake = Get-Command cmake.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    $ctest = Get-Command ctest.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $cmake -or -not $ctest) {
        $programRoots = @(
            [Environment]::GetEnvironmentVariable('ProgramW6432', 'Process')
            [Environment]::GetEnvironmentVariable('ProgramFiles', 'Process')
            [Environment]::GetEnvironmentVariable('ProgramFiles(x86)', 'Process')
            'C:\Program Files'
            'C:\Program Files (x86)'
        ) | Where-Object { $_ } | Select-Object -Unique
        $installations = @($programRoots | ForEach-Object {
            $visualStudio = Join-Path $_ 'Microsoft Visual Studio/2022'
            Get-ChildItem -LiteralPath $visualStudio -Directory -ErrorAction SilentlyContinue
        } | Sort-Object FullName -Unique)
        foreach ($installation in $installations) {
            $binary = Join-Path $installation.FullName 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin'
            if ((Test-Path -LiteralPath (Join-Path $binary 'cmake.exe') -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $binary 'ctest.exe') -PathType Leaf)) {
                $env:PATH = $binary + [IO.Path]::PathSeparator + $env:PATH
                break
            }
        }
        $cmake = Get-Command cmake.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        $ctest = Get-Command ctest.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    }
    if (-not $cmake -or -not $ctest) { throw 'CMake/CTest is required (PATH or the Visual Studio 2022 CMake installation).' }
    $flavor = if ($Qa) { 'qa' } elseif ($Unsigned) { 'unsigned' } else { 'release' }
    $buildId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [guid]::NewGuid().ToString('N')
    $work = Join-Path $PSScriptRoot "../output/windows-$flavor-work"
    $snapshot = [IO.Path]::GetFullPath((Join-Path $work "$buildId-source.json"))
    $manifest = [IO.Path]::GetFullPath((Join-Path $work "$buildId-build.json"))
    $evidenceTool = Join-Path $PSScriptRoot '../tools/mon_windows_build_evidence.py'
    & python $evidenceTool begin --output $snapshot
    if ($LASTEXITCODE -ne 0) { throw 'Source identity could not be recorded.' }
    if (-not $Qa) {
        # Windows PowerShell/native npm argument forwarding strips quotes in
        # inline JSON. Give Tauri an owned UTF-8 config file instead.
        New-Item -ItemType Directory -Path $work -Force | Out-Null
        $signingConfig = [IO.Path]::GetFullPath((Join-Path $work "$buildId-tauri-config.json"))
        [IO.File]::WriteAllText($signingConfig, $signing, [Text.UTF8Encoding]::new($false))
    }
    $tauriArgs = @('run', 'tauri', '--', 'build', '--target', 'x86_64-pc-windows-msvc',
        '--config', 'src-tauri/tauri.windows.bundle.conf.json')
    if ($Qa) { $tauriArgs += @('--config', 'src-tauri/tauri.qa.conf.json') }
    else { $tauriArgs += @('--config', $signingConfig) }
    $tauriArgs += @('--', '--locked')
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) { throw "Tauri build failed ($LASTEXITCODE)." }
    $baseConfig = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $productName = if ($Qa) { (Get-Content -LiteralPath 'src-tauri/tauri.qa.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json).productName } else { $baseConfig.productName }
    $installerName = '{0}_{1}_x64-setup.exe' -f $productName, $baseConfig.version
    $releaseDirectory = Join-Path $targetDirectory 'x86_64-pc-windows-msvc/release'
    $installer = Get-Item -LiteralPath (Join-Path $releaseDirectory "bundle/nsis/$installerName")
    if ($signedRelease) {
        $signature = Get-AuthenticodeSignature -LiteralPath $installer.FullName
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $env:MON_WINDOWS_CERTIFICATE_THUMBPRINT -or -not $signature.TimeStamperCertificate) { throw 'Public installer signature verification failed.' }
    }
    & $ctest.Source --test-dir (Join-Path $PSScriptRoot '../probes/windows-capture/build') -C Release --output-on-failure --no-tests=error
    if ($LASTEXITCODE -ne 0) { throw 'Native capture tests failed; previous installer retained.' }
    $nativeApp = Join-Path $releaseDirectory 'mon-desktop.exe'
    $captureHelper = Join-Path $PSScriptRoot 'src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe'
    if ($Unsigned) {
        $runtime = Join-Path $workspace '.cache/windows-runtime/package'
        $ownPaths = @($installer.FullName, $nativeApp, $captureHelper)
        foreach ($relative in @('mon-speaker-embedding.exe', 'workers/mon-whisper-worker.exe', 'workers/mon-llama-worker.exe',
            'workers/mon-sherpa-worker.exe', 'workers/mon-transducer-worker.exe', 'workers/mon-whisper-vulkan-worker.exe',
            'workers/mon-llama-vulkan-worker.exe', 'asr/bin/mon-transducer-worker.exe')) {
            $ownPaths += Join-Path $runtime $relative
        }
        $checks = @(& (Join-Path $workspace 'tools/assert-windows-unsigned.ps1') -Path $ownPaths)
        if (-not $?) { throw 'Unsigned artifact checks failed; previous installer retained.' }
        $checks | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $work "$buildId-unsigned.json") -Encoding UTF8
    }
    & python $evidenceTool finish --snapshot $snapshot --app $nativeApp --helper $captureHelper --installer $installer.FullName --output $manifest
    if ($LASTEXITCODE -ne 0) { throw 'Build identity verification failed; previous installer retained.' }
    & "$PSScriptRoot/../tools/publish-windows-installer.ps1" -Installer $installer.FullName -Flavor $flavor -BuildManifest $manifest
    Write-Output "Build manifest: $manifest"
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    Pop-Location
}
