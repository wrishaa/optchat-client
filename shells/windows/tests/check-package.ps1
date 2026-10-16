#Requires -Version 7.4
# Read-only package inspection: does not install the app or change certificate trust.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Package,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Architecture,
    [switch]$RequireSignature
)
. "$PSScriptRoot/../release/common.ps1"
$path = (Resolve-Path -LiteralPath $Package).Path
$checksum = (Get-Content -LiteralPath "$path.sha256" -Raw).Trim()
if ($checksum -notmatch '^([a-f0-9]{64})  (.+)$') { throw 'Malformed SHA256 sidecar.' }
if ($Matches[2] -cne [System.IO.Path]::GetFileName($path)) { throw 'SHA256 sidecar names a different file.' }
if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Matches[1]) { throw 'Package checksum mismatch.' }
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$unpack = Join-Path $root "build/windows/check-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory -Force $unpack
try {
    [System.IO.Compression.ZipFile]::ExtractToDirectory($path, $unpack)
    $isMsix = [System.IO.Path]::GetExtension($path) -eq '.msix'
    $app = if ($isMsix) { $unpack } else { Join-Path $unpack 'OptChat' }
    if ((Get-PeArchitecture "$app/optchat.exe") -ne $Architecture) { throw 'Package contains the wrong executable architecture.' }
    $info = Get-Content "$app/build-info.json" -Raw | ConvertFrom-Json
    if ($info.architecture -ne $Architecture -or -not $info.static_crt) { throw 'Build metadata mismatch.' }
    if (-not (Test-Path "$app/notices")) { throw 'Bundled notices are missing.' }
    if ($isMsix) {
        [xml]$manifest = Get-Content "$app/AppxManifest.xml" -Raw
        if ($manifest.Package.Identity.ProcessorArchitecture -ne $Architecture -or
            $manifest.Package.Identity.Name -ne 'Reframe.OptChat' -or
            $manifest.Package.Identity.Version -ne $info.version -or
            $manifest.Package.Identity.Publisher -cne $info.publisher) { throw 'Manifest and build metadata do not agree.' }
        foreach ($asset in @('StoreLogo.png', 'Square44x44Logo.png', 'Square150x150Logo.png')) {
            if (-not (Test-Path "$app/Assets/$asset")) { throw "Missing MSIX asset: $asset" }
        }
    }
    if ($RequireSignature) {
        Assert-Windows
        if ($isMsix) { Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', '/v', $path) }
        Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', '/v', "$app/optchat.exe")
    }
    Write-Host "PASS: checksum, $Architecture PE, metadata, notices and package layout."
    if (-not $RequireSignature) { Write-Host 'Signature trust was not checked. Native install and desktop qualification remain.' }
} finally { Remove-BuildDirectory $root $unpack }
