#Requires -Version 7.4
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')][string]$Architecture = 'x64',
    [ValidateSet('exe', 'zip', 'msix')][string[]]$Formats = @('exe'),
    [UInt16]$Revision = 0,
    [string]$Publisher = 'CN=OptChat Development',
    [string]$CertificateThumbprint,
    [string]$TimestampUrl,
    [string]$LicenseNotice,
    [switch]$Release
)
. "$PSScriptRoot/common.ps1"
Assert-Windows
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
if ($Release -and (-not $CertificateThumbprint -or -not $TimestampUrl -or -not $LicenseNotice -or $Publisher -eq 'CN=OptChat Development')) {
    throw 'Release needs a production Publisher, CertificateThumbprint, HTTPS TimestampUrl and approved LicenseNotice.'
}
if ($LicenseNotice) {
    $LicenseNotice = (Resolve-Path -LiteralPath $LicenseNotice).Path
    if ((Get-Item -LiteralPath $LicenseNotice).Length -eq 0) { throw 'License notice is empty.' }
}
$cert = Get-SigningCertificate $CertificateThumbprint $Publisher
if ($Release -and $cert.Subject -eq $cert.Issuer) { throw 'Production signing cannot use a self-signed certificate.' }
if ($TimestampUrl -and $TimestampUrl -notmatch '^https://') { throw 'Use an HTTPS RFC 3161 timestamp server.' }
$target = if ($Architecture -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'aarch64-pc-windows-msvc' }
foreach ($name in @('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', "CARGO_TARGET_$($target.ToUpperInvariant().Replace('-', '_'))_RUSTFLAGS")) {
    if ([Environment]::GetEnvironmentVariable($name)) { throw "Unset $name so the repository's static CRT flags take effect." }
}
# Run from the matching VS developer PowerShell (x64 or ARM64 target). This keeps toolchain changes explicit.
foreach ($tool in @('git', 'cargo', 'rustup', 'cmake', 'cl.exe', 'link.exe', 'dumpbin.exe')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool is missing; use a Visual Studio developer PowerShell with the C++ workload." }
}
$env:GPUI_FXC_PATH = Find-SdkTool 'fxc.exe'
$makeappx = if ('msix' -in $Formats) { Find-SdkTool 'makeappx.exe' } else { $null }
$innoCompiler = if ('exe' -in $Formats) { Find-InnoCompiler } else { $null }
Push-Location $root
$oldRevision = $env:OPTCHAT_WINDOWS_REVISION
try {
    & "$root/tools/zed-vendor.ps1"
    Invoke-Native rustup @('target', 'add', $target)
    $metadata = & cargo metadata --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read locked Cargo metadata.' }
    $package = ($metadata | ConvertFrom-Json).packages | Where-Object name -eq 'optchat-windows'
    $version = $package.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Use a three-part numeric workspace version for Windows packaging.' }
    $packageVersion = "$version.$Revision"
    if (@($packageVersion.Split('.') | Where-Object { [uint64]$_ -gt 65535 }).Count) { throw 'Version components must be <= 65535.' }
    if ($Release) { Assert-ReleaseSource $root $packageVersion }
    $output = Join-Path $root "$(if ($Release) { 'dist' } else { 'dist-dev' })/windows-$Architecture"
    $base = "OptChat-$packageVersion-windows-$Architecture"
    if (@(Get-ChildItem -Path "$output/$base-*" -File -ErrorAction SilentlyContinue).Count) {
        throw "Artifacts already exist for $base. Use a new revision."
    }
    $null = New-Item -ItemType Directory -Force $output
    $stageRoot = Join-Path $root "build/windows/$Architecture/$([guid]::NewGuid())"
    $stage = Join-Path $stageRoot 'OptChat'
    $null = New-Item -ItemType Directory -Force $stage
    # Separate targets ensure a host/Linux binary can never enter a Windows package.
    $targetDir = Join-Path $root 'target/windows'
    $env:OPTCHAT_WINDOWS_REVISION = [string]$Revision
    Invoke-Native cargo @('build', '--locked', '--release', '-p', 'optchat-windows', '--target', $target, '--target-dir', $targetDir)
    $binary = Join-Path $targetDir "$target/release/optchat.exe"
    if ((Get-PeArchitecture $binary) -ne $Architecture) { throw 'Executable architecture does not match the requested package.' }
    Copy-Item -LiteralPath $binary -Destination "$stage/optchat.exe"
    # Static CRT is intentional. Keep an import record and fail rather than shipping a hidden VC++ DLL dependency.
    $imports = & dumpbin.exe /DEPENDENTS $binary
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Windows DLL dependencies.' }
    Write-Utf8 "$stageRoot/imports.txt" ($imports -join "`n")
    if (($imports -join "`n") -match '(?i)\b(vcruntime\w*|msvcp\w*|concrt\w*)\.dll\b') {
        throw 'Unexpected dynamic VC++ runtime dependency. Correct static CRT linking before distributing.'
    }
    $notices = Join-Path $stage 'notices'
    $null = New-Item -ItemType Directory $notices
    Copy-Item "$root/assets/fonts/*LICENSE*", "$root/assets/fonts/*OFL*", "$root/assets/icons/LICENSE*" $notices
    if ($LicenseNotice) { Copy-Item -LiteralPath $LicenseNotice -Destination "$notices/PROJECT-NOTICE.txt" }
    else { Write-Utf8 "$notices/DEVELOPMENT-ONLY.txt" "Project/dependency license review is deferred. This is a development build, not approved for public distribution.`n" }
    if ($cert) { Sign-Artifact "$stage/optchat.exe" $CertificateThumbprint $TimestampUrl }
    if ($Release) { Assert-ProductionSignature "$stage/optchat.exe" $CertificateThumbprint }
    $commit = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read source revision.' }
    $info = [ordered]@{
        application = 'OptChat'; version = $packageVersion; architecture = $Architecture; target = $target
        source_commit = $commit; source_dirty = (@(& git status --porcelain --untracked-files=normal).Count -gt 0)
        gpui_commit = (& git -C vendor/zed rev-parse HEAD); rustc = (& rustc --version)
        built_utc = (Get-Date).ToUniversalTime().ToString('o'); static_crt = $true
        signed = [bool]$cert; publisher = $Publisher; release = [bool]$Release
        license_notice_supplied = [bool]$LicenseNotice
        license_notice_sha256 = if ($LicenseNotice) { (Get-FileHash -LiteralPath $LicenseNotice -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
    }
    Write-Utf8 "$stage/build-info.json" ($info | ConvertTo-Json)
    $artifacts = @()
    # The ZIP is an internal input unless explicitly requested as a public format.
    if ('zip' -in $Formats -or 'exe' -in $Formats) {
        $zipDirectory = if ('zip' -in $Formats) { $output } else { $stageRoot }
        $zip = Join-Path $zipDirectory "$base-portable.zip"
        if (Test-Path $zip) { throw "Output exists: $zip. Move it aside or increment Revision." }
        Compress-Archive -Path $stage -DestinationPath $zip
        Write-Checksum $zip
        if ('zip' -in $Formats) { $artifacts += $zip }
    }
    if ('exe' -in $Formats) {
        & "$PSScriptRoot/build-installer.ps1" -PortablePackage $zip -Architecture $Architecture `
            -Compiler $innoCompiler -CertificateThumbprint $CertificateThumbprint -TimestampUrl $TimestampUrl -OutputDirectory $output
        $artifacts += Join-Path $output "$base-Setup.exe"
    }
    if ('msix' -in $Formats) {
        $null = New-Item -ItemType Directory -Force "$stage/Assets"
        Copy-Item "$root/shells/windows/packaging/assets/*.png" "$stage/Assets"
        Write-PackageManifest "$root/shells/windows/packaging/AppxManifest.xml.in" "$stage/AppxManifest.xml" $Architecture $packageVersion $Publisher
        $suffix = if ($cert) { '' } else { '-unsigned' }
        $msix = Join-Path $output "$base$suffix.msix"
        if (Test-Path $msix) { throw "Output exists: $msix. Move it aside or increment Revision." }
        Invoke-Native $makeappx @('pack', '/d', $stage, '/p', $msix, '/h', 'SHA256', '/o')
        if ($cert) { Sign-Artifact $msix $CertificateThumbprint $TimestampUrl }
        if ($Release) { Assert-ProductionSignature $msix $CertificateThumbprint }
        Write-Checksum $msix
        $artifacts += $msix
    }
    $compilerVersion = if ($innoCompiler) { '6.7.3' } else { $null } # OptChat.iss checks the actual compiler version.
    Write-ReleaseManifest -Info $info -Artifacts $artifacts -Destination "$output/$base-release.json" -CompilerVersion $compilerVersion
    Write-Host "Packages saved to $output. Native install and desktop qualification are still required."
} finally { $env:OPTCHAT_WINDOWS_REVISION = $oldRevision; Pop-Location }
