#Requires -Version 7.4
function Initialize-AndroidBuild {
    param([string]$SdkRoot, [string]$JavaHome)
    $localTools = Join-Path $env:LOCALAPPDATA 'OptChat/AndroidTools'
    if (!$SdkRoot) { $SdkRoot = $env:ANDROID_HOME }
    if (!$SdkRoot) { $SdkRoot = Join-Path $localTools 'sdk' }
    if (!$JavaHome) { $JavaHome = $env:JAVA_HOME }
    if (!$JavaHome) {
        $jdk = Get-ChildItem -LiteralPath (Join-Path $localTools 'jdk') -Directory -ErrorAction SilentlyContinue |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin/java.exe') } |
            Sort-Object Name -Descending | Select-Object -First 1
        if ($jdk) { $JavaHome = $jdk.FullName }
    }
    if (!$JavaHome -or !(Test-Path -LiteralPath (Join-Path $JavaHome 'bin/java.exe'))) {
        throw 'Set JAVA_HOME to a JDK 17+ installation, or pass -JavaHome.'
    }
    $ndk = Join-Path $SdkRoot 'ndk/29.0.14206865'
    $ndkBin = Join-Path $ndk 'toolchains/llvm/prebuilt/windows-x86_64/bin'
    foreach ($file in @('platforms/android-36/android.jar', 'build-tools/36.0.0/aapt2.exe', 'platform-tools/adb.exe', 'ndk/29.0.14206865/source.properties')) {
        if (!(Test-Path -LiteralPath (Join-Path $SdkRoot $file))) { throw "Required SDK file is missing: $SdkRoot/$file" }
    }
    $env:JAVA_HOME = $JavaHome
    $env:ANDROID_HOME = $SdkRoot
    $env:ANDROID_NDK_HOME = $ndk
    $env:PATH = (Join-Path $env:USERPROFILE '.cargo/bin') + ';' + (Join-Path $JavaHome 'bin') + ';' + $env:PATH
    $env:CC_aarch64_linux_android = Join-Path $ndkBin 'aarch64-linux-android26-clang.cmd'
    $env:CXX_aarch64_linux_android = Join-Path $ndkBin 'aarch64-linux-android26-clang++.cmd'
    $env:AR_aarch64_linux_android = Join-Path $ndkBin 'llvm-ar.exe'
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = Join-Path $ndkBin 'clang.exe'
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = '-C link-arg=--target=aarch64-linux-android26 -C link-arg=-Wl,-z,max-page-size=16384'
}
