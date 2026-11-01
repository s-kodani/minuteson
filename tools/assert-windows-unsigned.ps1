param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string[]]$Path
)
# Read-only validation. Never strips signatures, signs files, or changes trust.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Unsigned PE verification requires Windows.' }
# Node/Python can inherit a PowerShell 7 PSModulePath before launching Windows
# PowerShell 5. Load this engine's built-in module instead of a foreign version.
Import-Module -Name (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1') -ErrorAction Stop

foreach ($candidate in $Path) {
    $file = Get-Item -LiteralPath $candidate -Force -ErrorAction Stop
    if ($file -isnot [IO.FileInfo] -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "An ordinary Windows executable file is required: $candidate"
    }
    # Keep a read-only handle that denies concurrent writing/replacement while
    # checking both the PE header and Windows' Authenticode result.
    $stream = [IO.File]::Open($file.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $reader = [IO.BinaryReader]::new($stream, [Text.Encoding]::ASCII, $true)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) {
            throw "Not a Windows PE executable: $($file.FullName)"
        }
        $stream.Position = 0x3c
        $peOffset = [long]$reader.ReadUInt32()
        if ($peOffset -lt 64 -or $peOffset -gt ($stream.Length - 24)) {
            throw "Invalid PE header offset: $($file.FullName)"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $($file.FullName)" }
        $null = $reader.ReadUInt16() # Machine; the NSIS bootstrapper can be x86.
        if ($reader.ReadUInt16() -eq 0) { throw "PE image has no sections: $($file.FullName)" }
        $stream.Position = $peOffset + 20
        $optionalSize = [long]$reader.ReadUInt16()
        $optionalStart = $peOffset + 24
        if ($optionalSize -lt 2 -or ($optionalStart + $optionalSize) -gt $stream.Length) {
            throw "Invalid PE optional header: $($file.FullName)"
        }
        $stream.Position = $optionalStart
        $magic = $reader.ReadUInt16()
        if ($magic -eq 0x10b) {
            $countOffset = 92
            $directoryOffset = 96
        } elseif ($magic -eq 0x20b) {
            $countOffset = 108
            $directoryOffset = 112
        } else {
            throw "Unsupported PE optional header: $($file.FullName)"
        }
        $securityOffset = $directoryOffset + 4 * 8
        if ($optionalSize -lt ($securityOffset + 8)) {
            throw "PE security directory is unavailable: $($file.FullName)"
        }
        $stream.Position = $optionalStart + $countOffset
        if ($reader.ReadUInt32() -lt 5) { throw "PE data directories are incomplete: $($file.FullName)" }
        $stream.Position = $optionalStart + $securityOffset
        $certificateOffset = $reader.ReadUInt32()
        $certificateSize = $reader.ReadUInt32()
        if ($certificateOffset -ne 0 -or $certificateSize -ne 0) {
            throw "Embedded certificate table exists; unsigned output is required: $($file.FullName)"
        }
        $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
            throw "Authenticode status must be NotSigned, got $($signature.Status): $($file.FullName)"
        }
        $stream.Position = 0
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $digest = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
        finally { $hasher.Dispose() }
        [pscustomobject]@{
            path = $file.FullName
            bytes = $stream.Length
            sha256 = $digest
            authenticodeStatus = 'NotSigned'
            embeddedCertificateTable = $false
        }
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}
