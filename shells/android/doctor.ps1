#Requires -Version 7.4
[CmdletBinding()]
param(
    [string]$Serial,
    [string]$SdkRoot = $env:ANDROID_HOME,
    [string]$JavaHome = $env:JAVA_HOME,
    [string]$AdbPath,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$localTools = Join-Path $env:LOCALAPPDATA 'OptChat/AndroidTools'
if (!$SdkRoot) {
    foreach ($candidate in @((Join-Path $localTools 'sdk'), (Join-Path $env:LOCALAPPDATA 'Android/Sdk'))) {
        if (Test-Path -LiteralPath $candidate -PathType Container) { $SdkRoot = $candidate; break }
    }
}
if (!$JavaHome) {
    $installedJdk = Get-ChildItem -LiteralPath (Join-Path $localTools 'jdk') -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin/java.exe') } |
        Sort-Object Name -Descending | Select-Object -First 1
    if ($installedJdk) { $JavaHome = $installedJdk.FullName }
}
if (!$AdbPath) {
    $adbCommand = Get-Command adb -ErrorAction SilentlyContinue
    if ($adbCommand) { $AdbPath = $adbCommand.Source }
    elseif ($SdkRoot -and (Test-Path -LiteralPath (Join-Path $SdkRoot 'platform-tools/adb.exe'))) {
        $AdbPath = Join-Path $SdkRoot 'platform-tools/adb.exe'
    }
}
if (!$AdbPath -or !(Test-Path -LiteralPath $AdbPath -PathType Leaf)) {
    throw 'ADB was not found. Install Android Platform Tools and pass -AdbPath or set ANDROID_HOME.'
}

function Invoke-Adb([string[]]$Arguments) {
    $lines = & $AdbPath @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "ADB failed: $($lines -join [Environment]::NewLine)" }
    return ($lines -join "`n").Trim()
}

$deviceList = Invoke-Adb @('devices', '-l')
$devices = @($deviceList -split "`r?`n" | ForEach-Object {
    if ($_ -match '^(\S+)\s+(device|unauthorized|offline)(?:\s|$)') {
        [pscustomobject]@{ serial = $Matches[1]; state = $Matches[2] }
    }
})
if ($Serial) {
    $selected = $devices | Where-Object serial -CEQ $Serial | Select-Object -First 1
    if (!$selected) { throw 'The requested device is not listed by ADB. Check its cable and USB debugging setting.' }
} else {
    if ($devices.Count -ne 1) { throw "ADB lists $($devices.Count) devices. Connect one device or pass -Serial." }
    $selected = $devices[0]
}
if ($selected.state -eq 'unauthorized') { throw 'Unlock the device and accept its Allow USB debugging prompt, then rerun.' }
if ($selected.state -ne 'device') { throw 'The selected device is offline. Reconnect it and rerun.' }
$deviceArgs = @('-s', $selected.serial)
function Get-DeviceProperty([string]$Name) { Invoke-Adb ($deviceArgs + @('shell', 'getprop', $Name)) }

$javaVersion = $null
if ($JavaHome -and (Test-Path -LiteralPath (Join-Path $JavaHome 'bin/java.exe'))) {
    $javaVersion = (& (Join-Path $JavaHome 'bin/java.exe') -version 2>&1 | ForEach-Object { "$_" }) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Java failed to start.' }
}
$rustup = Get-Command rustup -ErrorAction SilentlyContinue
if (!$rustup) {
    $rustupPath = Join-Path $env:USERPROFILE '.cargo/bin/rustup.exe'
    if (Test-Path -LiteralPath $rustupPath) { $rustup = Get-Item -LiteralPath $rustupPath }
}
$rustTargets = @()
if ($rustup) {
    $rustupExe = if ($rustup -is [System.IO.FileInfo]) { $rustup.FullName } else { $rustup.Source }
    $rustTargets = @(& $rustupExe target list --installed)
    if ($LASTEXITCODE -ne 0) { throw 'Could not query installed Rust targets.' }
}
function Get-SdkPackages([string]$Folder) {
    if (!$SdkRoot) { return @() }
    return @(Get-ChildItem -LiteralPath (Join-Path $SdkRoot $Folder) -Directory -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty Name)
}

$report = [ordered]@{
    schemaVersion = 1
    checkedAt = [DateTimeOffset]::UtcNow.ToString('o')
    device = [ordered]@{
        manufacturer = Get-DeviceProperty 'ro.product.manufacturer'
        model = Get-DeviceProperty 'ro.product.model'
        android = Get-DeviceProperty 'ro.build.version.release'
        api = [int](Get-DeviceProperty 'ro.build.version.sdk')
        abis = (Get-DeviceProperty 'ro.product.cpu.abilist') -split ','
        pageSize = [int](Invoke-Adb ($deviceArgs + @('shell', 'getconf', 'PAGE_SIZE')))
        displaySize = Invoke-Adb ($deviceArgs + @('shell', 'wm', 'size'))
    }
    tools = [ordered]@{
        adb = $AdbPath
        adbVersion = Invoke-Adb @('version')
        sdkRoot = $SdkRoot
        commandLineTools = @(Get-SdkPackages 'cmdline-tools')
        platforms = @(Get-SdkPackages 'platforms')
        buildTools = @(Get-SdkPackages 'build-tools')
        ndk = @(Get-SdkPackages 'ndk')
        javaHome = $JavaHome
        javaVersion = $javaVersion
        rustTargets = $rustTargets
        rustAndroidTargetInstalled = $rustTargets -contains 'aarch64-linux-android'
    }
    scope = 'Environment and device checks only; no app build or qualification.'
}
$json = $report | ConvertTo-Json -Depth 6
if ($OutputPath) {
    $absoluteOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $null = New-Item -ItemType Directory -Force -Path ([System.IO.Path]::GetDirectoryName($absoluteOutput))
    [System.IO.File]::WriteAllText($absoluteOutput, $json + [Environment]::NewLine)
}
$json
