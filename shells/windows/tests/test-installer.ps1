#Requires -Version 7.4
# Installs only into a unique test directory. Refuses to replace an existing EXE installation.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$Upgrade,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Architecture,
    [switch]$RequireSignature,
    [switch]$RequireProductionSignature,
    [switch]$SmokeTest
)
. "$PSScriptRoot/../release/common.ps1"
Assert-Windows
$first = (Resolve-Path -LiteralPath $Installer).Path
$second = (Resolve-Path -LiteralPath $Upgrade).Path
$firstVersion = [version](Get-Item -LiteralPath $first).VersionInfo.FileVersion.Trim()
$secondVersion = [version](Get-Item -LiteralPath $second).VersionInfo.FileVersion.Trim()
if ($secondVersion -le $firstVersion) { throw 'Upgrade must have a higher version than Installer.' }
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Reframe.OptChat.Desktop_is1'
if (Test-Path -LiteralPath $key) { throw 'An EXE installation already exists. Run this test on a disposable user profile.' }
foreach ($file in @($first, $second)) {
    $checksum = (Get-Content -LiteralPath "$file.sha256" -Raw).Trim()
    $expected = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($file)
    if ($checksum -cne $expected) { throw "Installer checksum mismatch: $file" }
    if ($RequireSignature) { Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', $file) }
    if ($RequireProductionSignature) {
        $signer = (Get-AuthenticodeSignature -LiteralPath $file).SignerCertificate
        if (-not $signer) { throw 'Installer is unsigned.' }
        Assert-ProductionSignature $file $signer.Thumbprint
    }
}
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$id = [guid]::NewGuid().ToString()
$scratch = Join-Path $root "build/windows/installer-test-$id"
$install = Join-Path $scratch 'OptChat test app'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'OptChat/OptChat.lnk'
if (Test-Path -LiteralPath (Split-Path -Parent $shortcut)) {
    throw 'An OptChat Start menu folder already exists. Run this test on a disposable user profile.'
}
$prefs = Join-Path $env:APPDATA 'OptChat'
$hadPrefs = Test-Path -LiteralPath $prefs
$canary = Join-Path $prefs "installer-test-$id.txt"
$null = New-Item -ItemType Directory -Force $scratch, $prefs
Write-Utf8 $canary $id

function Invoke-Setup {
    param([string]$Path, [string]$Log)
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/TASKS=',
        "/DIR=`"$install`"", "/LOG=`"$scratch/$Log`"")
    $process = Start-Process -FilePath $Path -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Setup failed with exit code $($process.ExitCode). See $scratch/$Log" }
}
function Assert-Installed {
    param([version]$Version)
    $registration = Get-ItemProperty -LiteralPath $key
    if ($registration.InstallLocation.TrimEnd('\') -ne $install.TrimEnd('\') -or
        [version]$registration.DisplayVersion -ne $Version) { throw 'Uninstall registration mismatch.' }
    if (-not (Test-Path -LiteralPath $shortcut)) { throw 'Start menu shortcut is missing.' }
    if ((Get-PeArchitecture "$install/optchat.exe") -ne $Architecture) { throw 'Installed architecture mismatch.' }
    $info = Get-Content "$install/build-info.json" -Raw | ConvertFrom-Json
    if ([version]$info.version -ne $Version) { throw 'Installed payload version mismatch.' }
    if ([version](Get-Item "$install/optchat.exe").VersionInfo.FileVersion.Trim() -ne $Version) {
        throw 'Application resource version differs from installer version.'
    }
    foreach ($name in @('imports.txt', 'INSTALL.txt')) {
        if (Test-Path -LiteralPath "$install/$name") { throw "Unnecessary file remains installed: $name" }
    }
    if ((Get-Content -LiteralPath $canary -Raw) -cne $id) { throw 'Installer changed existing preferences.' }
    if ($RequireSignature) {
        foreach ($name in @('optchat.exe', 'unins000.exe')) {
            Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', "$install/$name")
        }
    }
    if ($RequireProductionSignature) {
        $thumbprint = (Get-AuthenticodeSignature -LiteralPath $second).SignerCertificate.Thumbprint
        foreach ($name in @('optchat.exe', 'unins000.exe')) { Assert-ProductionSignature "$install/$name" $thumbprint }
    }
    Write-Host "PASS: installed $Version, architecture, Start shortcut, uninstall registration and retained preferences."
}
function Remove-TestInstallation {
    if (-not (Test-Path -LiteralPath $key)) { return }
    $registration = Get-ItemProperty -LiteralPath $key
    if ($registration.InstallLocation.TrimEnd('\') -ne $install.TrimEnd('\')) {
        throw 'Refusing to uninstall an installation outside the test directory.'
    }
    $uninstaller = Join-Path $install 'unins000.exe'
    $process = Start-Process -FilePath $uninstaller -WindowStyle Hidden -Wait -PassThru `
        -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$scratch/uninstall.log`"")
    if ($process.ExitCode -ne 0) { throw "Uninstall failed with exit code $($process.ExitCode)." }
    # Inno's temporary uninstaller finishes removing itself after the original process exits.
    $deadline = (Get-Date).AddSeconds(15)
    while ((Test-Path -LiteralPath $install) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
}
try {
    Invoke-Setup $first 'install.log'
    Assert-Installed $firstVersion
    # Simulate diagnostic files left by the previous packaging layout.
    Write-Utf8 "$install/imports.txt" 'old diagnostic'
    Write-Utf8 "$install/INSTALL.txt" 'old instructions'
    Invoke-Setup $second 'upgrade.log'
    Assert-Installed $secondVersion
    $rejected = Start-Process -FilePath $first -WindowStyle Hidden -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
        "/DIR=`"$install`"", "/LOG=`"$scratch/downgrade.log`"")
    if ($rejected.ExitCode -eq 0 -or (Get-Content "$scratch/downgrade.log" -Raw) -notmatch 'A newer version of OptChat is already installed') {
        throw 'Older installer was not rejected by the version guard.'
    }
    Assert-Installed $secondVersion
    Write-Host 'PASS: downgrade rejected without changing the installed version.'
    Invoke-Setup $second 'repair.log'
    Assert-Installed $secondVersion
    Write-Host 'PASS: same-version reinstall retained preferences.'
    if ($SmokeTest) {
        $oldBinary = $env:OPTCHAT_TEST_BINARY
        $oldInPlace = $env:OPTCHAT_TEST_IN_PLACE
        try {
            $env:OPTCHAT_TEST_BINARY = Join-Path $install 'optchat.exe'
            $env:OPTCHAT_TEST_IN_PLACE = '1'
            Invoke-Native 'deno' @('run', '-A', "$root/tests/windows.ts")
        } finally {
            $env:OPTCHAT_TEST_BINARY = $oldBinary
            $env:OPTCHAT_TEST_IN_PLACE = $oldInPlace
        }
    }
    Remove-TestInstallation
    if ((Test-Path -LiteralPath $key) -or (Test-Path -LiteralPath $shortcut) -or (Test-Path -LiteralPath "$install/optchat.exe")) {
        throw 'Uninstall left the app, shortcut or registration behind.'
    }
    if ((Get-Content -LiteralPath $canary -Raw) -cne $id) { throw 'Uninstall changed existing preferences.' }
    Write-Host "PASS: uninstall removed app, shortcut and registration; preferences retained. Logs: $scratch"
} finally {
    Remove-TestInstallation
    Remove-Item -LiteralPath $canary -ErrorAction SilentlyContinue
    if (-not $hadPrefs -and (Test-Path -LiteralPath $prefs) -and @(Get-ChildItem -LiteralPath $prefs -Force).Count -eq 0) {
        Remove-Item -LiteralPath $prefs
    }
}
