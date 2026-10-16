# Android maintainer guide

The Android app in `app/` contains a Rust `cdylib` (`optchat-android`) and a Gradle NativeActivity wrapper.
It uses the existing shared client/UI and GPUI pin; `vendor/gpui-mobile` is adapted by
`patches/gpui-mobile/android-app.patch`. `prepare-backend.ps1` applies or verifies exactly that patch.

## Build on Windows

Install JDK 21, Android SDK platform 36, build tools 36.0.0, NDK 29.0.14206865, Platform Tools and the
`aarch64-linux-android` Rust target. The Gradle 8.13 wrapper pins its download checksum; AGP is 8.13.0.
Rust uses the root toolchain file. Tool paths can be provided through `JAVA_HOME`, `ANDROID_HOME`,
`-JavaHome` and `-SdkRoot`. The scripts also recognize `%LOCALAPPDATA%\OptChat\AndroidTools`.

```powershell
./shells/android/doctor.ps1
./shells/android/build-app.ps1
```

The build prepares the submodules, runs Cargo with the lockfile, packages a stripped ARM64 library, checks
every ELF LOAD segment for at least 16 KB alignment, and verifies APK signing and ZIP page alignment.
The APK, SHA256 sidecar and build manifest go to `dist-dev/android-app/`. `-SkipNative` is for packaging
Java/resource changes against an existing current native library; rebuild normally after Rust changes.
Debug builds use the machine's Android debug keystore outside Git. Use `-VersionCode 2` (or a larger unused
integer) for a new build. The version name comes from `Cargo.toml` plus that code; Gradle, the APK filename
and build manifest receive the same values. Existing versioned outputs are never overwritten.

## CI and draft releases

The Android workflow builds ARM64 APKs on Windows with the pinned SDK/NDK and Gradle wrapper. Branch runs
use an automatically increasing development version code. Preview tags use `android-v<version>-preview.<code>`;
production candidate tags use `android-v<version>.<code>`. The version must match `Cargo.toml`, and the code
must be a new integer greater than all codes already distributed under the same application ID/signing key.
Retries of the same run/tag reuse the same code. Tag creation and pushing remain explicit maintainer actions.

Successful tagged builds create a **draft** GitHub Release with the APK, SHA256 checksum and build manifest.
They do not publish the draft or replace existing assets. Review device qualification before publishing.
Debug signatures from disposable CI runners are not stable update identities; use owner-signed candidates
for update qualification and distribution that must retain a signing identity.

For a production candidate, configure the protected `android-release` environment with required reviewers:

- Variables: `ANDROID_RELEASE_ENABLED=true` and `ANDROID_CERT_SHA256` (the release certificate fingerprint).
- Secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_STORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`.

The workflow decodes the keystore outside the checkout and removes it after building. The APK's signer
fingerprint must match the configured certificate. Keep the keystore backed up securely; no private signing
files belong in Git. A local `build-app.ps1 -VersionCode <code> -Release` uses the corresponding
`OPTCHAT_ANDROID_*` environment variables (see the script), a clean source tree and its exact production tag.
The release APK uses an optimized Rust build and disables Java/native debugging. Production signing does not
complete device, real-engine or the deferred notice review; drafts still need release approval.

Signing reference: [Android app signing](https://developer.android.com/studio/publish/app-signing).

## Device checks

Enable USB debugging and accept this computer on the device. Install the development APK with `adb install -r`.
The separate instrumentation APK uses native touch injection and Android's EditText InputConnection to check
engine token entry, Unicode/composition/UTF-16 selection, encrypted token storage and a chat round trip.
It uses the local fake engine only. Start with an empty development installation; the suite refuses connected
workspaces and must never run against real user engines.

```powershell
./shells/android/test-app.ps1
```

The suite starts a loopback fake engine, forwards its port over ADB and enables probes through an app-private
`files/test-mode` file in a debug build. Normal launches do not write probe/input data. Logs/screenshots go to
ignored `build/android/client-check/`. After checks, the suite removes its fake engine, test mode and test APK.
Do not enable test mode while entering real credentials. The installed OptChat app stays available afterwards.

## Remaining qualification

See [QUALIFICATION.md](QUALIFICATION.md). Real-engine use, microphone capture/STT, file selection, camera/QR,
accessibility, long background/process-death behavior, multiple Android versions/devices, and 16 KB runtime
compatibility need separate checks. API 26 is the declared minimum, not a tested support baseline.
The backend uses available system Bengali fonts and bitmap emoji; fallback coverage varies by device.
The project's deferred license/notice review and production distribution requirements still apply.
