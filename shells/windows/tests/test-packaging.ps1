#Requires -Version 7.4
# Runs on Linux or Windows. Fixtures are synthetic packaging inputs, never app build artifacts.
. "$PSScriptRoot/../release/common.ps1"
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$scripts = @(Get-ChildItem "$root/shells/windows" -Filter '*.ps1' -Recurse)
$scripts += Get-Item "$root/tools/zed-vendor.ps1"
foreach ($script in $scripts) {
    $tokens = $null; $errors = $null
    $null = [System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "$($script.FullName): $($errors -join '; ')" }
}
$scratch = Join-Path $root "build/windows/fixtures-$([guid]::NewGuid())"
$app = Join-Path $scratch 'OptChat'
$null = New-Item -ItemType Directory -Force "$app/notices"
function Expect-Failure {
    param([scriptblock]$Action)
    $failed = $false
    try { & $Action } catch { $failed = $true }
    if (-not $failed) { throw 'An invalid package input was accepted.' }
}
try {
    foreach ($arch in @('x64', 'arm64')) {
        $bytes = [byte[]]::new(256)
        [BitConverter]::GetBytes([UInt16]0x5a4d).CopyTo($bytes, 0)
        [BitConverter]::GetBytes([int]0x80).CopyTo($bytes, 0x3c)
        [BitConverter]::GetBytes([UInt32]0x4550).CopyTo($bytes, 0x80)
        $machine = if ($arch -eq 'x64') { 0x8664 } else { 0xaa64 }
        [BitConverter]::GetBytes([UInt16]$machine).CopyTo($bytes, 0x84)
        [IO.File]::WriteAllBytes("$app/optchat.exe", $bytes)
        if ((Get-PeArchitecture "$app/optchat.exe") -ne $arch) { throw 'PE architecture detection failed.' }
        Write-PackageManifest "$root/shells/windows/packaging/AppxManifest.xml.in" "$scratch/AppxManifest.xml" $arch '0.1.0.42' 'CN=Test & Co'
        [xml]$manifest = Get-Content "$scratch/AppxManifest.xml" -Raw
        if ($manifest.Package.Identity.Publisher -cne 'CN=Test & Co' -or $manifest.Package.Identity.ProcessorArchitecture -ne $arch) { throw 'Manifest substitution failed.' }
        $min = if ($arch -eq 'arm64') { '10.0.22000.0' } else { '10.0.19045.0' }
        if ($manifest.Package.Dependencies.TargetDeviceFamily.MinVersion -ne $min) { throw 'OS minimum mismatch.' }
        Write-Utf8 "$app/build-info.json" (@{architecture=$arch; static_crt=$true; version='0.1.0.42'; publisher='CN=Test & Co'} | ConvertTo-Json)
        Write-Utf8 "$app/notices/fixture.txt" 'Synthetic test fixture'
        $zip = Join-Path $scratch "$arch.zip"
        Compress-Archive -Path $app -DestinationPath $zip
        Write-Checksum $zip
        & "$PSScriptRoot/check-package.ps1" -Package $zip -Architecture $arch
        $wrongArch = if ($arch -eq 'x64') { 'arm64' } else { 'x64' }
        Expect-Failure { & "$PSScriptRoot/check-package.ps1" -Package $zip -Architecture $wrongArch }
        # Exercise MSIX layout without claiming MakeAppx schema/signing validation on Linux.
        Copy-Item "$scratch/AppxManifest.xml" $app
        $null = New-Item -ItemType Directory -Force "$app/Assets"
        Copy-Item "$root/shells/windows/packaging/assets/*.png" "$app/Assets" -Force
        $msix = Join-Path $scratch "$arch.msix"
        [IO.Compression.ZipFile]::CreateFromDirectory($app, $msix)
        Write-Checksum $msix
        & "$PSScriptRoot/check-package.ps1" -Package $msix -Architecture $arch
        Remove-Item -LiteralPath "$app/AppxManifest.xml"
        Remove-BuildDirectory $root "$app/Assets"
        [IO.File]::AppendAllText($zip, 'corruption')
        Expect-Failure { & "$PSScriptRoot/check-package.ps1" -Package $zip -Architecture $arch }
    }
    [IO.File]::WriteAllBytes("$app/optchat.exe", [byte[]]::new(256))
    Expect-Failure { Get-PeArchitecture "$app/optchat.exe" }
    Expect-Failure { Write-PackageManifest "$root/shells/windows/packaging/AppxManifest.xml.in" "$scratch/bad.xml" 'x86' '0.1.0.0' 'CN=Test' }
    Expect-Failure { Write-PackageManifest "$root/shells/windows/packaging/AppxManifest.xml.in" "$scratch/bad.xml" 'x64' '0.1.0-beta' 'CN=Test' }
    Expect-Failure { Write-PackageManifest "$root/shells/windows/packaging/AppxManifest.xml.in" "$scratch/bad.xml" 'x64' '0.1.0.65536' 'CN=Test' }
    $manifestPath = Join-Path $scratch 'release.json'
    Write-ReleaseManifest -Info @{version='0.1.0.42'; release=$false; source_dirty=$true} `
        -Artifacts @("$scratch/x64.zip", "$scratch/arm64.zip") -Destination $manifestPath -CompilerVersion 'fixture'
    $release = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($release.schema_version -ne 1 -or $release.artifacts.Count -ne 2 -or -not $release.build.source_dirty) {
        throw 'Release manifest lost artifact or source information.'
    }
    foreach ($artifact in $release.artifacts) {
        $file = Get-Item -LiteralPath (Join-Path $scratch $artifact.file)
        if ($artifact.bytes -ne $file.Length -or $artifact.sha256 -cne (Get-FileHash $file.FullName).Hash.ToLowerInvariant()) {
            throw 'Release manifest did not hash the actual output bytes.'
        }
    }
    $source = Join-Path $scratch 'source'
    $null = New-Item -ItemType Directory $source
    Invoke-Native git @('init', '--quiet', $source)
    Write-Utf8 "$source/source.txt" 'fixture'
    Invoke-Native git @('-C', $source, 'add', 'source.txt')
    Invoke-Native git @('-C', $source, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'fixture')
    Expect-Failure { Assert-ReleaseSource $source '0.1.0.42' }
    Invoke-Native git @('-C', $source, 'tag', 'windows-v0.1.0.42')
    Assert-ReleaseSource $source '0.1.0.42'
    Write-Utf8 "$source/untracked.txt" 'must not release'
    Expect-Failure { Assert-ReleaseSource $source '0.1.0.42' }
    Remove-Item -LiteralPath "$source/untracked.txt"
    Write-Utf8 "$source/source.txt" 'modified'
    Expect-Failure { Assert-ReleaseSource $source '0.1.0.42' }
    Expect-Failure { Remove-BuildDirectory $root $root }
    Write-Host 'PASS: generated artifact hashes, release tag/clean-source guards and cleanup boundary.'
    Write-Host 'PASS: PowerShell parsing, both architecture layouts, manifest escaping/minimum OS, wrong-architecture and corruption rejection.'
} finally { Remove-BuildDirectory $root $scratch }
