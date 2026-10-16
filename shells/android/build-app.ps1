#Requires -Version 7.4
[CmdletBinding()]
param(
    [string]$SdkRoot,
    [string]$JavaHome,
    [string]$GradlePath,
    [switch]$SkipNative,
    [ValidateRange(1, 2100000000)][int]$VersionCode = 1,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $PSScriptRoot 'build-env.ps1')
Initialize-AndroidBuild -SdkRoot $SdkRoot -JavaHome $JavaHome

Push-Location $root
try {
    & (Join-Path $root 'tools/zed-vendor.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Could not prepare pinned GPUI.' }
    # zed-vendor.ps1 prepares and verifies both workspace backends.
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve workspace version.' }
    $baseVersion = (($metadataText | ConvertFrom-Json).packages | Where-Object name -eq 'optchat-android').version
    if ($baseVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected a numeric Cargo workspace version.' }
    $versionName = if ($Release) { "$baseVersion.$VersionCode" } else { "$baseVersion-dev.$VersionCode" }
    $variant = if ($Release) { 'release' } else { 'debug' }
    $name = "OptChat-$versionName-arm64.apk"
    $output = Join-Path $root "$(if ($Release) { 'dist' } else { 'dist-dev' })/android-app"
    if (Test-Path (Join-Path $output $name)) { throw 'Installer already exists; increment VersionCode.' }
    if ($Release) {
        if ($SkipNative) { throw 'Release builds cannot reuse an unverified native library.' }
        if (@(& git status --porcelain).Count) { throw 'Release builds require committed, clean source.' }
        $tags = @(& git tag --points-at HEAD)
        if ($LASTEXITCODE -ne 0 -or "android-v$baseVersion.$VersionCode" -cnotin $tags) { throw 'Release needs its exact version tag at HEAD.' }
        foreach ($setting in @('OPTCHAT_ANDROID_KEYSTORE', 'OPTCHAT_ANDROID_STORE_PASSWORD', 'OPTCHAT_ANDROID_KEY_ALIAS', 'OPTCHAT_ANDROID_KEY_PASSWORD', 'OPTCHAT_ANDROID_CERT_SHA256')) {
            if (-not [Environment]::GetEnvironmentVariable($setting)) { throw "Configure $setting for signing." }
        }
    }
    if (!$SkipNative) {
        $cargoArguments = @('build', '--locked', '-p', 'optchat-android', '--target', 'aarch64-linux-android', '-j', '2')
        if ($Release) { $cargoArguments += '--release' }
        & cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) { throw 'Rust Android build failed.' }
    }
    $targetDirectory = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $root 'target' }
    $profile = if ($Release) { 'release' } else { 'debug' }
    $native = Join-Path $targetDirectory "aarch64-linux-android/$profile/liboptchat_android.so"
    if (!(Test-Path -LiteralPath $native)) { throw 'Build the native OptChat library first.' }
    $jniDirectory = Join-Path $root 'build/android/app/jniLibs/arm64-v8a'
    $null = New-Item -ItemType Directory -Force -Path $jniDirectory
    Copy-Item -LiteralPath $native -Destination $jniDirectory
    & (Join-Path $env:ANDROID_NDK_HOME 'toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-strip.exe') --strip-debug (Join-Path $jniDirectory 'liboptchat_android.so')
    if ($LASTEXITCODE -ne 0) { throw 'Could not strip packaged debug symbols.' }
    $programHeaders = & (Join-Path $env:ANDROID_NDK_HOME 'toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-readelf.exe') -lW (Join-Path $jniDirectory 'liboptchat_android.so')
    if ($LASTEXITCODE -ne 0) { throw 'Could not read native ELF program headers.' }
    $loadHeaders = @($programHeaders | Where-Object { $_ -match '^\s*LOAD\s' })
    if (!$loadHeaders.Count) { throw 'Native library has no ELF load segments.' }
    foreach ($header in $loadHeaders) {
        $alignment = ($header.Trim() -split '\s+')[-1]
        if ([Convert]::ToInt64($alignment, 16) -lt 16384) { throw 'Native ELF segment alignment is smaller than 16 KB.' }
    }
    if (!$GradlePath) {
        $localGradle = Join-Path $env:LOCALAPPDATA 'OptChat/AndroidTools/gradle-8.13/bin/gradle.bat'
        $GradlePath = if (Test-Path -LiteralPath $localGradle) { $localGradle } else { Join-Path $PSScriptRoot 'app/gradlew.bat' }
    }
    $apk = Join-Path $PSScriptRoot "app/app/build/outputs/apk/$variant/app-$variant.apk"
    # A new APK avoids unused payload left behind by incremental ZIP updates after a native rebuild.
    if (Test-Path -LiteralPath $apk) { Remove-Item -LiteralPath $apk }
    $task = if ($Release) { 'assembleRelease' } else { 'assembleDebug' }
    & $GradlePath -p (Join-Path $PSScriptRoot 'app') $task --console=plain "-PoptchatVersionCode=$VersionCode" "-PoptchatVersionName=$versionName"
    if ($LASTEXITCODE -ne 0) { throw 'APK assembly failed.' }
    $signature = & (Join-Path $env:ANDROID_HOME 'build-tools/36.0.0/apksigner.bat') verify --verbose --print-certs $apk
    if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed.' }
    $certificate = [regex]::Match(($signature -join "`n"), 'Signer #1 certificate SHA-256 digest: ([a-fA-F0-9]{64})').Groups[1].Value.ToLowerInvariant()
    if (-not $certificate) { throw 'APK signer certificate fingerprint is missing.' }
    if ($Release -and $certificate -cne $env:OPTCHAT_ANDROID_CERT_SHA256.Replace(':', '').ToLowerInvariant()) {
        throw 'APK is not signed by the configured production certificate.'
    }
    $badging = & (Join-Path $env:ANDROID_HOME 'build-tools/36.0.0/aapt2.exe') dump badging $apk
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect packaged Android version.' }
    $packageLine = ($badging | Where-Object { $_ -match '^package:' }) -join ''
    if ($packageLine -notmatch "versionCode='$VersionCode'" -or $packageLine -notmatch "versionName='$([regex]::Escape($versionName))'") {
        throw 'Packaged Android version differs from the requested version.'
    }
    & (Join-Path $env:ANDROID_HOME 'build-tools/36.0.0/zipalign.exe') -c -P 16 -v 4 $apk
    if ($LASTEXITCODE -ne 0) { throw 'APK page alignment verification failed.' }
    $null = New-Item -ItemType Directory -Force -Path $output
    $package = Join-Path $output $name
    Copy-Item -LiteralPath $apk -Destination $package
    $hash = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$package.sha256", "$hash  $name`n")
    $archive = [IO.Compression.ZipFile]::OpenRead($package)
    try {
        $entry = $archive.GetEntry('lib/arm64-v8a/liboptchat_android.so')
        if (!$entry) { throw 'APK is missing the OptChat native library.' }
        $stream = $entry.Open()
        try { $packagedNativeHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() }
        finally { $stream.Dispose() }
    } finally { $archive.Dispose() }
    $sourceHashes = [ordered]@{}
    $sourceFiles = @(
        Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'app/src'), (Join-Path $PSScriptRoot 'app/app/src/main') -File -Recurse
        Get-Item -LiteralPath (Join-Path $PSScriptRoot 'app/Cargo.toml'), (Join-Path $PSScriptRoot 'app/app/build.gradle.kts'),
            (Join-Path $root 'crates/client/Cargo.toml'), (Join-Path $root 'crates/client/src/mic.rs'),
            (Join-Path $root 'crates/platform/src/lib.rs')
    )
    foreach ($file in $sourceFiles | Sort-Object FullName) {
        $relative = [IO.Path]::GetRelativePath($root, $file.FullName).Replace('\', '/')
        $sourceHashes[$relative] = (Get-FileHash -LiteralPath $file.FullName).Hash.ToLowerInvariant()
    }
    $metadata = [ordered]@{
        kind = if ($Release) { 'signed OptChat Android release candidate' } else { 'development OptChat Android client' }
        applicationId = 'com.reframe.optchat.gpui.android'
        versionCode = $VersionCode
        versionName = $versionName
        signing = if ($Release) { 'owner-provided release keystore' } else { 'Android debug keystore; development only' }
        release = [bool]$Release
        signerCertificateSha256 = $certificate
        abi = 'arm64-v8a'
        sourceCommit = (& git rev-parse HEAD)
        workingTreeDirty = [bool](@(& git status --porcelain).Count)
        appSourceSha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'app/src/lib.rs')).Hash.ToLowerInvariant()
        sources = $sourceHashes
        nativeLibrarySha256 = $packagedNativeHash
        nativeInputLibrarySha256 = (Get-FileHash -LiteralPath (Join-Path $jniDirectory 'liboptchat_android.so')).Hash.ToLowerInvariant()
        cargoLockSha256 = (Get-FileHash -LiteralPath (Join-Path $root 'Cargo.lock')).Hash.ToLowerInvariant()
        rust = (& rustc --version)
        gpui = '97dc71d8b8f79012c1441aa01121fba95667292a'
        backend = 'c7cab3a43970bd5f1e05d907695404ed73fcdc95'
        patchSha256 = (Get-FileHash -LiteralPath (Join-Path $root 'patches/gpui-mobile/android-app.patch')).Hash.ToLowerInvariant()
        ndk = '29.0.14206865'
        sdk = 36
        buildTools = '36.0.0'
        nativeElfPageAlignment = 'at least 16384 bytes for every LOAD segment'
        gradle = '8.13'
        agp = '8.13.0'
        artifact = $name
        sha256 = $hash
        builtAt = [DateTimeOffset]::UtcNow.ToString('o')
    }
    [IO.File]::WriteAllText((Join-Path $output 'build-info.json'), ($metadata | ConvertTo-Json -Depth 5) + "`n")
    [IO.File]::WriteAllText((Join-Path $output "$name.build.json"), ($metadata | ConvertTo-Json -Depth 5) + "`n")
    Write-Output "Built $package"
} finally { Pop-Location }
