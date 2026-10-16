#Requires -Version 7.4
# Wrap an already built portable package; never rebuild Rust or install tools.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PortablePackage,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Architecture,
    [string]$Compiler,
    [string]$CertificateThumbprint,
    [string]$TimestampUrl,
    [string]$OutputDirectory
)
. "$PSScriptRoot/common.ps1"
Assert-Windows
$compilerPath = Find-InnoCompiler $Compiler
$packagePath = (Resolve-Path -LiteralPath $PortablePackage).Path
if ([System.IO.Path]::GetExtension($packagePath) -ne '.zip') { throw 'Supply a portable ZIP from build.ps1.' }
& "$PSScriptRoot/../tests/check-package.ps1" -Package $packagePath -Architecture $Architecture
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$stage = Join-Path $root "build/windows/installer-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory -Force $stage
try {
    [System.IO.Compression.ZipFile]::ExtractToDirectory($packagePath, $stage)
    $payload = Join-Path $stage 'OptChat'
    $info = Get-Content "$payload/build-info.json" -Raw | ConvertFrom-Json
    $version = [string]$info.version
    if ($version -notmatch '^\d+\.\d+\.\d+\.\d+$' -or
        @($version.Split('.') | Where-Object { [uint64]$_ -gt 65535 }).Count -gt 0) {
        throw 'Installer version must have four numeric components, each <= 65535.'
    }
    $cert = Get-SigningCertificate $CertificateThumbprint $info.publisher
    if ($TimestampUrl -and $TimestampUrl -notmatch '^https://[^\s"$]+$') {
        throw 'Use an HTTPS RFC 3161 timestamp URL without whitespace, quotes or dollar signs.'
    }
    if ($info.release -and (-not $cert -or -not $TimestampUrl -or -not $info.signed -or $info.source_dirty -or
        $info.publisher -eq 'CN=OptChat Development' -or
        -not (Test-Path "$payload/notices/PROJECT-NOTICE.txt") -or
        (Test-Path "$payload/notices/DEVELOPMENT-ONLY.txt"))) {
        throw 'A release installer needs a signed release payload, approved notices and timestamped production signing.'
    }
    if ($cert) {
        Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', "$payload/optchat.exe")
        $payloadSignature = Get-AuthenticodeSignature -LiteralPath "$payload/optchat.exe"
        if ($payloadSignature.SignerCertificate.Thumbprint -ne $CertificateThumbprint) {
            throw 'Installer and payload must use the same signing certificate.'
        }
    }
    if ($info.release) { Assert-ProductionSignature "$payload/optchat.exe" $CertificateThumbprint }
    $output = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Split-Path -Parent $packagePath }
    $null = New-Item -ItemType Directory -Force $output
    $setup = Join-Path $output "OptChat-$version-windows-$Architecture-Setup.exe"
    if (Test-Path $setup) { throw "Output exists: $setup. Use a new package revision." }
    $arguments = @('/Qp', "/DPayloadDir=$payload", "/DAppVersion=$version", "/DArchitecture=$Architecture", "/DOutputDir=$output")
    if ($cert) {
        # Inno invokes signtool directly; $q/$f are its documented quoting/file placeholders.
        $signTool = (Find-SdkTool 'signtool.exe').Replace('$', '$$')
        $signCommand = '$q' + $signTool + '$q sign /fd SHA256 /sha1 ' + $CertificateThumbprint + ' /s My'
        if ($TimestampUrl) { $signCommand += ' /tr $q' + $TimestampUrl + '$q /td SHA256' }
        $signCommand += ' $f'
        $arguments += @('/DSignInstaller=1', "/SOptChatSign=$signCommand")
    }
    Invoke-Native $compilerPath ($arguments + @("$root/shells/windows/packaging/OptChat.iss"))
    if (-not (Test-Path $setup)) { throw 'Compiler did not produce the expected installer.' }
    if ($cert) { Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', $setup) }
    if ($info.release) { Assert-ProductionSignature $setup $CertificateThumbprint }
    Write-Checksum $setup
    Write-ReleaseManifest -Info $info -Artifacts @($setup) `
        -Destination (Join-Path $output "OptChat-$version-windows-$Architecture-release.json") `
        -CompilerVersion '6.7.3' # Enforced by OptChat.iss; ISCC.exe's resource version is 0.0.0.0.
    Write-Host "Installer: $setup. Install/upgrade/uninstall qualification is required before distribution."
} finally {
    Remove-BuildDirectory $root $stage
}
