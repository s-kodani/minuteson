param([switch]$ValidateOnly)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build MSIX packages on Windows.' }
$command = if ($ValidateOnly) { 'validate' } else { 'build' }
& python (Join-Path $PSScriptRoot '../tools/mon_windows_msix.py') $command
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
