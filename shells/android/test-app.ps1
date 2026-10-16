#Requires -Version 7.4
[CmdletBinding()]
param([string]$Serial, [int]$Port = 18790, [string]$Apk, [string]$SdkRoot, [string]$JavaHome)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $PSScriptRoot 'build-env.ps1')
Initialize-AndroidBuild -SdkRoot $SdkRoot -JavaHome $JavaHome
$adb = Join-Path $env:ANDROID_HOME 'platform-tools/adb.exe'
$package = 'com.reframe.optchat.gpui.android'
$testPackage = "$package.test"
$component = "$package/.OptChatActivity"
$output = Join-Path $root 'build/android/client-check'
$null = New-Item -ItemType Directory -Force -Path $output
if (!$Apk) {
    $latest = Get-Content (Join-Path $root 'dist-dev/android-app/build-info.json') -Raw | ConvertFrom-Json
    $Apk = Join-Path $root "dist-dev/android-app/$($latest.artifact)"
}
if (!(Test-Path -LiteralPath $Apk)) { throw 'Run build-app.ps1 first.' }
$deno = (Get-Command deno.exe -ErrorAction Stop).Source
$devices = @(& $adb devices | Select-Object -Skip 1 | Where-Object { $_ -match '^\S+\s+device$' } | ForEach-Object { ($_ -split '\s+')[0] })
if (!$Serial) {
    if ($devices.Count -ne 1) { throw 'Connect one authorized Android device, or pass -Serial.' }
    $Serial = $devices[0]
}
if ($Serial -notin $devices) { throw 'The selected Android device is not authorized/connected.' }
$selector = @('-s', $Serial)
function Adb {
    $result = & $adb @selector @args
    if ($LASTEXITCODE -ne 0) { throw "ADB failed: $($args[0])" }
    $result
}
function Read-AppJson([string]$name) {
    (Adb exec-out run-as $package cat "files/$name") -join "`n" | ConvertFrom-Json
}
function State { Read-AppJson 'probes.json.state.json' }
function Wait-For([string]$label, [scriptblock]$condition) {
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    while ([DateTime]::UtcNow -lt $deadline) {
        try { if (& $condition) { return } } catch { }
        Start-Sleep -Milliseconds 250
    }
    throw "Timed out: $label"
}
$densityLine = (Adb shell wm density) | Select-Object -Last 1
$scale = [double]([regex]::Match($densityLine, '(\d+)\s*$').Groups[1].Value) / 160
function Tap-Probe([string]$name) {
    $bounds = (Read-AppJson 'probes.json').$name
    if (!$bounds) { throw "Missing visible probe: $name" }
    Adb shell input tap ([int](($bounds[0] + $bounds[2] / 2) * $scale)) ([int](($bounds[1] + $bounds[3] / 2) * $scale)) | Out-Null
    Start-Sleep -Milliseconds 600
}
function Launch { Adb shell am start -W -n $component | Out-Null }
function Screenshot([string]$name) { & $adb @selector exec-out screencap -p > (Join-Path $output $name); if ($LASTEXITCODE) { throw 'Screenshot failed.' } }

# Refuse to overwrite or test an installation carrying user engines.
if (Adb shell pm path $package) {
    Adb shell run-as $package pwd | Out-Null
    & $adb @selector shell run-as $package test -f files/prefs.json
    if ($LASTEXITCODE -eq 0) {
        $prefs = Read-AppJson 'prefs.json'
        if (@($prefs.engines).Count -or @($prefs.connections).Count) { throw 'Use an empty development installation; existing engines must be removed by their owner first.' }
    }
}
if ((Adb reverse --list) -match "tcp:$Port\s") { throw 'This ADB reverse port is already in use.' }
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
try { $listener.Start() } finally { $listener.Stop() }
$gradle = Join-Path $env:LOCALAPPDATA 'OptChat/AndroidTools/gradle-8.13/bin/gradle.bat'
if (!(Test-Path -LiteralPath $gradle)) { $gradle = Join-Path $PSScriptRoot 'app/gradlew.bat' }
& $gradle -p (Join-Path $PSScriptRoot 'app') assembleDebugAndroidTest --console=plain *> (Join-Path $output 'test-build.log')
if ($LASTEXITCODE) { throw 'Instrumentation APK build failed.' }
$engine = $null
$startedTests = $false
$result = [ordered]@{ result = 'INCOMPLETE'; checkedAt = [DateTimeOffset]::UtcNow.ToString('o'); apkSha256 = (Get-FileHash -LiteralPath $Apk).Hash.ToLowerInvariant() }
try {
    $engine = Start-Process -FilePath $deno -ArgumentList @('run', "--allow-net=127.0.0.1:$Port", ('"' + (Join-Path $root 'tests/lib/fake-engine.ts') + '"'), "$Port", 'alpha', 'beta', '--echo') -WorkingDirectory $root -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output 'engine.log') -RedirectStandardError (Join-Path $output 'engine-error.log')
    Wait-For 'isolated fake engine' { !$engine.HasExited -and (Invoke-RestMethod "http://127.0.0.1:$Port/engine/workspaces").Count -eq 2 }
    Adb install -r $Apk | Out-File (Join-Path $output 'install.log')
    Adb install -r (Join-Path $PSScriptRoot 'app/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk') | Out-File (Join-Path $output 'test-install.log')
    Adb shell run-as $package touch files/test-mode | Out-Null
    Adb reverse "tcp:$Port" "tcp:$Port" | Out-Null
    $startedTests = $true
    Adb shell am force-stop $package | Out-Null
    $testOutput = Adb shell am instrument -w -e port "$Port" "$testPackage/com.reframe.optchat.gpui.android.OptChatDeviceChecks"
    $testOutput | Out-File (Join-Path $output 'device-tests.log')
    if (($testOutput -join "`n") -notmatch 'PASS: (\d+) Android device checks') { throw 'Device instrumentation checks failed; see device-tests.log.' }
    $result.deviceChecks = [int]$Matches[1]
    Adb shell am force-stop $package | Out-Null
    Adb shell run-as $package rm -f files/probes.json.state.json | Out-Null
    Launch
    Wait-For 'saved tokens restore both workspaces' { $state = State; $state.loaded -and $state.connected.Count -eq 2 -and $state.engineSockets -eq 1 }
    $result.tokenRestorationAfterRelaunch = $true
    Screenshot 'chat.png'
    for ($cycle = 0; $cycle -lt 3; $cycle++) {
        Adb shell input keyevent KEYCODE_HOME | Out-Null
        Start-Sleep -Milliseconds 600
        Launch
        Wait-For 'chat after Home/resume' { (State).loaded }
        Tap-Probe 'phone.tab.more'
        Wait-For 'More screen accepts touch' { (State).phone.screen -eq 'more' }
        Adb shell input keyevent KEYCODE_BACK | Out-Null
        Wait-For 'Android Back returns to chat' { $null -eq (State).phone.screen }
    }
    $result.homeResumeCycles = 3
    $result.androidBack = $true
    $result.result = 'PASS'
} catch {
    $result.result = 'FAIL'
    $result.error = $_.Exception.Message
    throw
} finally {
    if ($startedTests) {
        try {
            Launch
            Wait-For 'test state' { (State).phone }
            $prefs = Read-AppJson 'prefs.json'
            if (@($prefs.engines).Count) {
                if ($prefs.engines.Count -ne 1 -or $prefs.engines[0].endpoint -ne "http://127.0.0.1:$Port" -or $prefs.engines[0].name -ne 'Android device check') { throw 'Unexpected engines present; refusing automatic removal.' }
                $screen = (State).phone.screen
                if ($screen -and $screen -ne 'workspaces') {
                    Tap-Probe 'phone.back'
                    Wait-For 'return from test panel' { $null -eq (State).phone.screen }
                }
                if ((State).phone.screen -ne 'workspaces') { Tap-Probe 'phone.avatar' }
                Wait-For 'test Workspaces screen' { (State).phone.screen -eq 'workspaces' }
                Tap-Probe 'wsp.remove.0'
                Wait-For 'fake engine removed' { @((Read-AppJson 'prefs.json').engines).Count -eq 0 }
            }
            Adb shell am force-stop $package | Out-Null
            Adb shell run-as $package rm -f files/test-mode files/probes.json files/probes.json.tmp files/probes.json.cmd files/probes.json.state.json files/probes.json.state.json.tmp | Out-Null
            Adb reverse --remove "tcp:$Port" | Out-Null
            Adb uninstall $testPackage | Out-Null
            Launch
            Start-Sleep -Seconds 1
            Screenshot 'ready-to-connect.png'
            $result.testDataRemoved = $true
        } catch { $result.result = 'FAIL'; $result.cleanupError = $_.Exception.Message; Write-Warning $result.cleanupError }
    }
    if ($engine -and !$engine.HasExited) { Stop-Process -Id $engine.Id; $engine.WaitForExit() }
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'result.json')
}
if ($result.cleanupError) { throw 'Device checks need cleanup; see result.json.' }
Write-Output "Passed $($result.deviceChecks) device checks, token restoration, three Home/resume cycles and Android Back. OptChat is ready to connect."
