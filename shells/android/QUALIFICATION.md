# OptChat Android development qualification

Date: **2026-10-16** (Asia/Dhaka). The shared OptChat client and phone UI are installed on the owner's
OnePlus 9 5G. The hello-window package and the separate instrumentation package have been uninstalled.
OptChat is left on Workspaces with no engine configured. The artifact was built from working changes on
`fabiha/android-packages`.

## Package and source

- [ARM64 APK](../../releases/android/OptChat-0.1.0-dev-arm64.apk): 32,245,819 bytes, development/debug signing.
- SHA256: `abd928f9c99e4907cdf77164d2a9168cb844c4283c456ba99e62fa0e5eaa7d20`.
- Application ID `com.reframe.optchat.gpui.android`, version code 1, version `0.1.0-dev`.
- Declared minimum API 26, compile/target API 36. Device checks cover Android 14 / API 34 only.
- [Build manifest](../../releases/android/build-info.json): source/lockfile/patch hashes, actual packaged native
  library hash and tool versions. Built from working changes over `6b748dc`, rather than a new release commit.
- GPUI remains at `97dc71d8b8f79012c1441aa01121fba95667292a`.
- `gpui-mobile` remains at `c7cab3a43970bd5f1e05d907695404ed73fcdc95`, with
  `patches/gpui-mobile/android-app.patch` applied and verified by `prepare-backend.ps1`.

## Checks

Device: OnePlus LE2115, Android 14, ARM64, 1080 × 2400, density 3, 4096-byte pages; Vulkan / Adreno 660.

| Check | Result |
|---|---|
| Native shared client/UI build and Gradle assembly | Pass |
| APK signature, native ELF and ZIP 16 KB alignment | Pass |
| Artifact, packaged native library and recorded source hashes | Pass |
| Backend patch preparation | Pass on a fresh sparse checkout and repeat run; unexpected edits rejected and verification preserves the real Git index |
| Android lint | Pass; advisory warnings remain for synchronous preference deletion, ChromeOS ABI coverage and legacy backup metadata |
| Instrumented device checks | **25 pass** using actual touch injection and Android EditText InputConnection |
| Engine connection | Two fake workspaces share one engine socket |
| Input | Token/name entry, Bengali/emoji, IME composition, UTF-16 selection/replacement; stale IME connections rejected after field changes |
| Chat | Typed message sent to the isolated fake engine; its response rendered; composer cleared |
| Secure storage | Keystore encryption/decryption, no plaintext token in encrypted preferences or app prefs, deletion |
| Relaunch | Both workspaces restored through the saved encrypted engine token |
| Home/resume | Three cycles; touch and Android Back return from More to chat after each |
| Visual inspection | Bengali and bitmap emoji render correctly; status/navigation icons readable; clean Workspaces screen |
| Final cleanup | Fake engine/token, test-mode/probe files and instrumentation APK removed; hello-window app uninstalled |
| Shared unit tests | 21 proto and 8 theme tests pass; one theme test ignored |

The reproducible device suite is [test-app.ps1](test-app.ps1). Its final
[result](evidence/oneplus9-result.json) matches the APK above. The
[chat screenshot](evidence/oneplus9-chat.png) contains only fake-engine data; the
[ready screen](evidence/oneplus9-ready.png) shows the app left for the owner.
Raw device logs retain the phone's clock, which differs from the build host; report timestamps use host UTC.
An earlier run was interrupted by USB disconnection. The final connected-device run passed completely.

## Limits

This qualifies a development installation on one phone. Real-engine access, microphone capture/STT, file
selection, camera/QR, accessibility, rotation/process death, prolonged background use, large-history performance
and battery behavior remain unqualified. Microphone permission is requested on use; QR scanning is unavailable.
Fonts depend on available Android system fallbacks, including a compatible legacy bitmap emoji font.
Other Android versions/devices and actual 16 KB runtime compatibility need checks. Alignment on this 4 KB
phone does not establish the declared API-26 baseline or 16 KB runtime support.

Production signing, Google Play publication and the project's deferred license/redistribution review are pending.
