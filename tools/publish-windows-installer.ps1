param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [ValidateSet('qa', 'release')][string]$Flavor = 'qa',
    [string]$OutputRoot
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\', '/')
if (-not $OutputRoot) { $OutputRoot = Join-Path $workspace 'output' }
function Assert-WorkspacePath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Installer publication must stay inside the workspace: $full"
    }
    $cursor = $full
    while ($cursor.Length -gt $workspace.Length) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Installer publication cannot follow a link: $cursor"
            }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    return $full
}
function Get-Sha256([string]$Path) {
    $hash = [Security.Cryptography.SHA256]::Create()
    $hashStream = [IO.File]::OpenRead($Path)
    try { return [BitConverter]::ToString($hash.ComputeHash($hashStream)).Replace('-', '') }
    finally { $hashStream.Dispose(); $hash.Dispose() }
}
$source = Assert-WorkspacePath $Installer
$root = Assert-WorkspacePath $OutputRoot
$folder = if ($Flavor -eq 'qa') { 'windows-qa' } else { 'windows-release' }
$distribution = Assert-WorkspacePath (Join-Path $root $folder)
$work = Assert-WorkspacePath (Join-Path $root "$folder-work")
$name = if ($Flavor -eq 'qa') { 'MON-QA-setup.exe' } else { 'MON-setup.exe' }
$latest = Assert-WorkspacePath (Join-Path $distribution $name)
if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or [IO.Path]::GetExtension($source) -ne '.exe') {
    throw 'A built Windows installer executable is required.'
}
$stream = [IO.File]::OpenRead($source)
try {
    if ($stream.Length -lt 2 -or $stream.ReadByte() -ne 0x4d -or $stream.ReadByte() -ne 0x5a) {
        throw 'The source is not a Windows executable.'
    }
} finally { $stream.Dispose() }
$expectedHash = Get-Sha256 $source
New-Item -ItemType Directory -Path $distribution, $work -Force | Out-Null
$stage = Assert-WorkspacePath (Join-Path $work ('.installer-' + [guid]::NewGuid().ToString('N') + '.tmp'))
try {
    Copy-Item -LiteralPath $source -Destination $stage
    if ((Get-Sha256 $stage) -ne $expectedHash) { throw 'Installer copy verification failed.' }
    $obsolete = @()
    foreach ($entry in @(Get-ChildItem -LiteralPath $distribution -Force)) {
        $entryPath = Assert-WorkspacePath $entry.FullName
        if ($entry.Name -eq $name) {
            if ($entry.PSIsContainer) { throw 'The installer destination is a directory.' }
            continue
        }
        # Only generated MON installers may be deleted. Preserve diagnostics,
        # checksums, scripts and any unfamiliar file outside the download folder.
        if (-not $entry.PSIsContainer -and $entry.Name -match '^(?:MON-QA-.+-setup|MON(?: QA)?_.+_x64-setup|MON-setup)\.exe$') {
            $obsolete += $entryPath
            continue
        }
        if ($entry.PSIsContainer) {
            foreach ($child in Get-ChildItem -LiteralPath $entryPath -Recurse -Force) {
                $null = Assert-WorkspacePath $child.FullName
            }
        }
        $archive = Assert-WorkspacePath (Join-Path $work $entry.Name)
        if (Test-Path -LiteralPath $archive) {
            $archive = Assert-WorkspacePath (Join-Path $work ($entry.Name + '.' + [guid]::NewGuid().ToString('N')))
        }
        Move-Item -LiteralPath $entryPath -Destination $archive
    }
    if (Test-Path -LiteralPath $latest) {
        [IO.File]::Replace($stage, $latest, [NullString]::Value)
    } else {
        Move-Item -LiteralPath $stage -Destination $latest
    }
    if ((Get-Sha256 $latest) -ne $expectedHash) { throw 'Published installer verification failed.' }
    foreach ($old in $obsolete) { Remove-Item -LiteralPath (Assert-WorkspacePath $old) }
    $remaining = @(Get-ChildItem -LiteralPath $distribution -Force)
    if ($remaining.Count -ne 1 -or $remaining[0].Name -ne $name) { throw 'The download folder must contain only the current installer.' }
    Write-Output "Installer: $latest"
    Write-Output "SHA256: $expectedHash"
    Write-Output "Build and verification files: $work"
} finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath (Assert-WorkspacePath $stage) }
}
