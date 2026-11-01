param(
    [Parameter(Mandatory=$true)][string]$BuildWorkspace,
    [Parameter(Mandatory=$true)][string]$BuildManifest,
    [Parameter(Mandatory=$true)][string]$ExpectedCommit,
    [string]$StorePackage,
    [string]$OutputDirectory,
    [string]$CertificateThumbprint,
    [switch]$CreateCertificate
)
# Only copies/signs local output. Never trusts a certificate, installs a
# package, stops an application, changes app data, or deletes a signing key.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Prepare the local test copy on Windows.' }
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $root ('output/windows-store-test/' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [guid]::NewGuid().ToString('N'))
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$helper = Join-Path $PSScriptRoot 'mon_windows_msix_test.py'
$arguments = @($helper, 'prepare', '--build-workspace', $BuildWorkspace, '--build-manifest', $BuildManifest,
    '--expected-commit', $ExpectedCommit, '--output', $OutputDirectory)
if ($StorePackage) { $arguments += @('--store-package', $StorePackage) }
& python @arguments
if ($LASTEXITCODE -ne 0) { throw 'Unsigned Store build/copy verification failed; no certificate was created and nothing was signed.' }
$planPath = Join-Path $OutputDirectory 'test-plan.json'
$plan = Get-Content -LiteralPath $planPath -Raw -Encoding UTF8 | ConvertFrom-Json
$friendlyName = 'MinutesOn MSIX local test (persistent)'

function Assert-TestSigner($Certificate) {
    if (-not $Certificate.HasPrivateKey -or $Certificate.Subject -cne $plan.identity.publisher `
        -or $Certificate.FriendlyName -cne $friendlyName `
        -or $Certificate.NotBefore -gt (Get-Date) -or $Certificate.NotAfter -le (Get-Date).AddHours(1)) {
        throw 'Persistent test certificate identity, private key, friendly name, or validity is unsuitable.'
    }
    $usage = @($Certificate.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.15' })
    $eku = @($Certificate.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.37' })
    if ($usage.Count -ne 1 -or $usage[0].KeyUsages -ne [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature `
        -or $eku.Count -ne 1 -or @($eku[0].EnhancedKeyUsages | Where-Object { $_.Value -eq '1.3.6.1.5.5.7.3.3' }).Count -ne 1) {
        throw 'Persistent test certificate must declare DigitalSignature and Code Signing EKU.'
    }
    $key = [Security.Cryptography.X509Certificates.RSACertificateExtensions]::GetRSAPrivateKey($Certificate)
    try {
        if ($key -isnot [Security.Cryptography.RSACng] -or $key.Key.ExportPolicy -ne [Security.Cryptography.CngExportPolicies]::None) {
            throw 'The persistent test key must be a nonexportable CNG RSA key.'
        }
    } finally { if ($key) { $key.Dispose() } }
}

$created = $false
if ($CertificateThumbprint) {
    if ($CertificateThumbprint -notmatch '^[0-9a-fA-F]{40}$') { throw 'Supply an exact certificate thumbprint.' }
    $certificate = Get-Item -LiteralPath ('Cert:\CurrentUser\My\' + $CertificateThumbprint)
    Assert-TestSigner $certificate
} else {
    $candidates = @(Get-ChildItem Cert:\CurrentUser\My | Where-Object {
        $_.Subject -ceq $plan.identity.publisher -and $_.FriendlyName -ceq $friendlyName `
        -and $_.HasPrivateKey -and $_.NotBefore -le (Get-Date) -and $_.NotAfter -gt (Get-Date).AddHours(1)
    })
    if ($candidates.Count -gt 1) { throw 'Multiple persistent test certificates match. Select one explicitly with -CertificateThumbprint; no new key was created.' }
    if ($candidates.Count -eq 1) {
        $certificate = $candidates[0]
        Assert-TestSigner $certificate
    } elseif ($CreateCertificate) {
        $certificate = New-SelfSignedCertificate -Type Custom -KeyUsage DigitalSignature `
            -Provider 'Microsoft Software Key Storage Provider' -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 `
            -KeyExportPolicy NonExportable -CertStoreLocation 'Cert:\CurrentUser\My' `
            -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3','2.5.29.19={text}') `
            -Subject $plan.identity.publisher -FriendlyName $friendlyName -NotAfter (Get-Date).AddDays(90)
        $created = $true
        Assert-TestSigner $certificate
    } else {
        throw 'No reusable persistent test signer exists. Earlier public certificates cannot sign without their deleted private keys. Review and use -CreateCertificate once; later builds reuse that key.'
    }
}

$cerPath = Join-Path $OutputDirectory 'test-signing-public.cer'
Export-Certificate -Cert $certificate -FilePath $cerPath -NoClobber | Out-Null
$publicCertificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($cerPath)
try {
    if ($publicCertificate.HasPrivateKey -or $publicCertificate.Thumbprint -ne $certificate.Thumbprint `
        -or $publicCertificate.Subject -cne $plan.identity.publisher) { throw 'Exported public certificate differs from the selected signer.' }
    & $plan.signtool sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $plan.testPackage
    if ($LASTEXITCODE -ne 0) { throw 'Local test-copy signing failed; persistent test key was retained.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    Add-Type -AssemblyName System.Security
    $archive = [IO.Compression.ZipFile]::OpenRead($plan.testPackage)
    try {
        $stream = $archive.GetEntry('AppxSignature.p7x').Open()
        $memory = New-Object IO.MemoryStream
        try { $stream.CopyTo($memory); $bytes = $memory.ToArray() }
        finally { $stream.Dispose(); $memory.Dispose() }
        if ($bytes.Length -le 4 -or [Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'PKCX') { throw 'Unexpected MSIX signature format.' }
        $cms = New-Object Security.Cryptography.Pkcs.SignedCms
        $cms.Decode([byte[]]$bytes[4..($bytes.Length - 1)])
        $cms.CheckSignature($true)
        if ($cms.SignerInfos.Count -ne 1 -or $cms.SignerInfos[0].Certificate.Thumbprint -ne $publicCertificate.Thumbprint `
            -or $cms.SignerInfos[0].Certificate.Subject -cne $publicCertificate.Subject `
            -or $cms.ContentInfo.ContentType.Value -ne '1.3.6.1.4.1.311.2.1.4') { throw 'MSIX CMS signer/Publisher/content type mismatch.' }
        $content = $cms.ContentInfo.Content
        $text = [Text.Encoding]::ASCII.GetString($content)
        $offset = $text.IndexOf('APPX', [StringComparison]::Ordinal)
        if ($offset -lt 0 -or ($content.Length - $offset) -ne 184 `
            -or $text.IndexOf('APPX', $offset + 4, [StringComparison]::Ordinal) -ge 0) { throw 'Unexpected signed APPX SHA-256 hash block.' }
        $signedHashes = @{}
        for ($position = $offset + 4; $position -lt $content.Length; $position += 36) {
            $tag = [Text.Encoding]::ASCII.GetString($content, $position, 4)
            if ($signedHashes.ContainsKey($tag)) { throw 'Duplicate signed metadata hash tag.' }
            $signedHashes[$tag] = [BitConverter]::ToString($content, $position + 4, 32).Replace('-', '').ToLowerInvariant()
        }
        if ($signedHashes.Count -ne 5 -or @('AXPC','AXCD','AXCT','AXBM','AXCI' | Where-Object { -not $signedHashes.ContainsKey($_) }).Count) { throw 'Missing signed metadata hash tags.' }
        $metadataHashes = [ordered]@{}
        foreach ($item in @(@{tag='AXCI'; name='AppxMetadata/CodeIntegrity.cat'}, @{tag='AXBM'; name='AppxBlockMap.xml'}, @{tag='AXCT'; name='[Content_Types].xml'})) {
            $entry = $archive.GetEntry($item.name)
            if (-not $entry -or $entry.Length -le 0) { throw ('Missing signed metadata: ' + $item.name) }
            $stream = $entry.Open(); $hasher = [Security.Cryptography.SHA256]::Create()
            try { $digest = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
            finally { $stream.Dispose(); $hasher.Dispose() }
            if ($digest -ne $signedHashes[$item.tag]) { throw ('CMS metadata digest mismatch: ' + $item.name) }
            $metadataHashes[$item.tag] = $digest
        }
    } finally { $archive.Dispose() }
    if (-not (Get-Item -LiteralPath ('Cert:\CurrentUser\My\' + $certificate.Thumbprint)).HasPrivateKey) { throw 'The persistent test key is unexpectedly missing.' }
    $reportPath = Join-Path $OutputDirectory 'signature-verification.json'
    [ordered]@{
        signatureCryptoVerified=$true; windowsTrustVerified=$false; metadataHashes=$metadataHashes
        certificate=[ordered]@{
            thumbprint=$certificate.Thumbprint; subject=$certificate.Subject; friendlyName=$certificate.FriendlyName
            publicCertificate=$cerPath; publicCertificateSha256=(Get-FileHash -LiteralPath $cerPath -Algorithm SHA256).Hash.ToLowerInvariant()
            notBefore=$certificate.NotBefore.ToUniversalTime().ToString('o'); notAfter=$certificate.NotAfter.ToUniversalTime().ToString('o')
            createdByPreparation=$created; persistentKeyRetained=$true; privateKeyExported=$false; nonExportable=$true
        }
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    & python $helper finish --plan $planPath --signature-report $reportPath
    if ($LASTEXITCODE -ne 0) { throw 'Signed local test verification failed; preserve outputs and persistent key for investigation.' }
    Write-Output ('Public certificate: ' + $cerPath)
    Write-Output 'Persistent test key retained for reuse. No trust was imported, no app was stopped, and no package was installed.'
} finally { $publicCertificate.Dispose() }
