#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$X64Package,
    [Parameter(Mandatory)][string]$Arm64Package,
    [string]$Publisher = 'CN=OptChat Development',
    [string]$CertificateThumbprint,
    [string]$TimestampUrl,
    [switch]$Release
)
. "$PSScriptRoot/common.ps1"
Assert-Windows
if ($Release -and (-not $CertificateThumbprint -or -not $TimestampUrl -or $Publisher -eq 'CN=OptChat Development')) {
    throw 'Release bundles need production signing and an HTTPS timestamp server.'
}
$cert = Get-SigningCertificate $CertificateThumbprint $Publisher
if ($TimestampUrl -and $TimestampUrl -notmatch '^https://') { throw 'Use an HTTPS RFC 3161 timestamp server.' }
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$stage = Join-Path $root "build/windows/bundle-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory $stage -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem
$identities = @()
foreach ($inputPackage in @($X64Package, $Arm64Package)) {
    $path = (Resolve-Path -LiteralPath $inputPackage).Path
    $archive = [System.IO.Compression.ZipFile]::OpenRead($path)
    try {
        $entry = $archive.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw "Missing manifest in $path" }
        $reader = [System.IO.StreamReader]::new($entry.Open())
        try { $identities += ([xml]$reader.ReadToEnd()).Package.Identity } finally { $reader.Dispose() }
        if ($Release) {
            $infoEntry = $archive.GetEntry('build-info.json')
            if (-not $infoEntry) { throw 'Release package is missing build metadata.' }
            $reader = [System.IO.StreamReader]::new($infoEntry.Open())
            try { $info = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
            if (-not $info.release) { throw 'Build each input with -Release before making a release bundle.' }
        }
    } finally { $archive.Dispose() }
    Copy-Item -LiteralPath $path -Destination $stage
}
$first, $second = $identities
if ($first.Name -ne 'Reframe.OptChat' -or $second.Name -ne $first.Name -or
    $first.Publisher -cne $Publisher -or $second.Publisher -cne $Publisher -or
    $first.Version -ne $second.Version -or $first.ProcessorArchitecture -ne 'x64' -or
    $second.ProcessorArchitecture -ne 'arm64') { throw 'Bundle requires matching OptChat x64/ARM64 identities, publishers and versions.' }
if ($Release) {
    foreach ($inputPackage in @($X64Package, $Arm64Package)) {
        $signature = Get-AuthenticodeSignature -LiteralPath $inputPackage
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $CertificateThumbprint) {
            throw 'Release bundles require validly signed input packages from the same release certificate.'
        }
    }
}
$output = Join-Path $root "$(if ($Release) { 'dist' } else { 'dist-dev' })/windows-bundle"
$null = New-Item -ItemType Directory -Force $output
$suffix = if ($cert) { '' } else { '-unsigned' }
$bundle = Join-Path $output "OptChat-$($first.Version)-windows$suffix.msixbundle"
if (Test-Path $bundle) { throw "Output exists: $bundle" }
Invoke-Native (Find-SdkTool 'makeappx.exe') @('bundle', '/d', $stage, '/p', $bundle, '/o')
if ($cert) { Sign-Artifact $bundle $CertificateThumbprint $TimestampUrl }
Write-Checksum $bundle
Write-Host "Bundle saved to $bundle"
