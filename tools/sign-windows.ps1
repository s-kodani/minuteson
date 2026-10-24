param(
    [string]$Path,
    [string]$CertificateThumbprint = $env:MON_WINDOWS_CERTIFICATE_THUMBPRINT,
    [string]$TimestampUrl = $env:MON_WINDOWS_TIMESTAMP_URL,
    [switch]$ValidateOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Windows signing requires Windows.' }
if ($CertificateThumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw 'Specify the exact public-release certificate thumbprint in MON_WINDOWS_CERTIFICATE_THUMBPRINT.' }
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$CertificateThumbprint" -ErrorAction Stop
$now = Get-Date
if (-not $certificate.HasPrivateKey -or $certificate.NotBefore -gt $now -or $certificate.NotAfter -le $now) { throw 'The specified signing certificate is unavailable, expired, or has no private key.' }
$eku = @($certificate.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId.Value })
if ($eku -notcontains '1.3.6.1.5.5.7.3.3') { throw 'The specified certificate does not permit code signing.' }
$timestamp = $null
if (-not [Uri]::TryCreate($TimestampUrl, [UriKind]::Absolute, [ref]$timestamp) -or $timestamp.Scheme -ne 'https') { throw 'Specify the certificate provider HTTPS RFC 3161 endpoint in MON_WINDOWS_TIMESTAMP_URL.' }
$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (-not $signtool) {
    $kit = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction Stop
    $candidate = Get-ChildItem -LiteralPath (Join-Path $kit.KitsRoot10 'bin') -Directory |
        Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $candidate) { throw 'The Windows SDK signing tool is missing.' }
    $signtool = Get-Command -Name $candidate
}
if ($ValidateOnly) { return }
$binary = (Get-Item -LiteralPath $Path -ErrorAction Stop).FullName
if ([IO.Path]::GetExtension($binary) -notin @('.exe', '.dll')) { throw 'Only an explicit EXE or DLL can be signed.' }
& $signtool.Source sign /sha1 $CertificateThumbprint /s My /fd SHA256 /tr $TimestampUrl /td SHA256 $binary
if ($LASTEXITCODE -ne 0) { throw 'Authenticode signing or timestamping failed.' }
& $signtool.Source verify /pa /all /tw $binary
if ($LASTEXITCODE -ne 0) { throw 'Authenticode verification failed.' }
$signature = Get-AuthenticodeSignature -LiteralPath $binary
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $CertificateThumbprint -or -not $signature.TimeStamperCertificate) { throw 'The verified signature, expected certificate, or RFC 3161 timestamp is missing.' }
