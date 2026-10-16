#Requires -Version 7.4
[CmdletBinding()]
param([string]$OutputDirectory = "$PSScriptRoot/../../../build/windows/test-signing")
. "$PSScriptRoot/common.ps1"
Assert-Windows
$null = New-Item -ItemType Directory -Force $OutputDirectory
$cert = New-SelfSignedCertificate -Type Custom -Subject 'CN=OptChat Development' `
    -CertStoreLocation 'Cert:/CurrentUser/My' -KeyAlgorithm RSA -KeyLength 2048 `
    -HashAlgorithm SHA256 -KeyExportPolicy NonExportable -KeyUsage DigitalSignature `
    -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3') -NotAfter (Get-Date).AddMonths(3)
$path = Join-Path (Resolve-Path $OutputDirectory).Path 'OptChat-Development.cer'
$null = Export-Certificate -Cert $cert -FilePath $path
Write-Host "Development certificate created. Public certificate: $path"
Write-Host 'Trust is not changed. Reuse this thumbprint for upgrade tests; never use this identity for production.'
Write-Output $cert.Thumbprint
