# Android implementation research

Investigation date: 2026-10-15. Branch: `fabiha/android-packages`, created from the completed Windows packaging
branch at `6b748dc`. The owner selected the GPUI Android backend approach and a hello-window experiment before
the app port. The experiment was completed and has been replaced by the shared OptChat client and UI.
See the [Android qualification](QUALIFICATION.md) for the current app, tested artifact and device evidence.

## Recommendation

Proceed with a GPUI Android backend against our existing pin, starting with a small hello-window APK on the
OnePlus before porting OptChat. The comparison below records the alternatives considered; Kotlin is not the
selected implementation. Android performance and product readiness still need device evidence.

The [root README](../../README.md#android-gpui-spike-app-port-pending) explicitly
calls for the comparison and spike before an app port. Its boundary rule also keeps OS APIs inside platform
shells, and its dependency policy keeps every shell on one GPUI pin. The existing [iOS research](../../docs/ios-plan.md)
records a GPUI decision after measurement; its iOS results do not establish Android performance.

## What the choice changes

| | GPUI Android shell | Native Kotlin UI |
|---|---|---|
| Code potentially retained | Rust protocol, client state/networking, themes and existing phone UI once a compatible backend works | Protocol contract and fixtures; Rust protocol helpers only if an explicit binding is added |
| Main new work | Android platform backend, Java/Kotlin host, IME/touch/lifecycle integration, secure token store and shell hooks | Android screens, chat rendering, client state/reducer, connection integration and platform services |
| Framework integration | Adapt a community backend to `97dc71d`, carrying patches; changing the shared pin would require checking the other platforms | Avoids a GPUI Android backend; retaining the Rust client needs a separate GUI-free core and bindings |
| Ongoing ownership | Shared app UI plus Android framework glue and carried patches | A second UI/client implementation plus protocol parity checks |
| Performance evidence here | None yet | None yet |
| Installer | Signed APK for direct installation; AAB for a Play distribution workflow | The same APK/AAB choices |

Kotlin does not automatically reuse `optchat-client`: its state is a GPUI entity. `optchat-proto` is Rust,
so consuming its code from Kotlin also needs bindings. A separate Kotlin protocol implementation can instead
be checked against common JSON fixtures. Both approaches still require Android packaging and device testing.
Claims that either one will be faster, use less battery, or be smaller need measurements.

## Backend findings

OptChat pins Zed at `97dc71d8b8f79012c1441aa01121fba95667292a`. Local inspection confirms that
`vendor/zed/crates/gpui_platform/src/gpui_platform.rs` has no Android constructor and there is no `gpui_android`
crate at that pin. Android-capable dependencies and renderer surface handling alone do not provide a backend.

| Candidate | Evidence reviewed | Implication |
|---|---|---|
| Upstream Android work | [Mobile tracker](https://github.com/mikayla-maki/gpui-projects/blob/main/projects/mobile.md), [Android issue #12](https://github.com/mikayla-maki/gpui-projects/issues/12), and PR search | Android remains listed as later work in the tracker. No supported Android backend was found for our pin. |
| `itsbalamurali/gpui-mobile` | [Revision `c7cab3a`](https://github.com/itsbalamurali/gpui-mobile/tree/c7cab3a43970bd5f1e05d907695404ed73fcdc95), manifest, Android platform/window/dispatcher/JNI sources and example | A concrete candidate for the first spike. Uses NativeActivity, `gpui_wgpu` and cosmic-text. Pins Zed `5688167d224b5eca54875d49afb8bfd73a07915a`, so it is not a drop-in dependency. |
| Zedra's `gpui_android` | [Zedra README](https://github.com/deltaqdev/zedra), its submodule declaration, and [backend revision `5955915`](https://github.com/tanlethanh/zed/tree/5955915c572ea9336d9648a2d286b399298fe5ff/crates/gpui_android) | An existing Android app and a useful host/input reference. Its fork's core APIs differ from ours; replacing the complete Zed submodule would affect every platform. |
| Zed PR #61057 | [PR and closure](https://github.com/zed-industries/zed/pull/61057), GitHub API status, commit summaries | Closed without merging on 2026-08-08. Contains an Android example, IME and Keystore work, but is not upstream support. Its code has not been compiled or audited in this investigation. |

Concrete compatibility and product gaps found by source inspection:

- Our `Platform::on_quit` callback returns `bool`. Both inspected community platform implementations take a
  callback returning `()`. Integration therefore needs source adaptation, even before device testing.
- Zedra's window code imports `PointerKind`, `SelectableTextHitRegion` and `SelectionMenuPresentation`, which
  are absent from our pinned GPUI sources. Its Android crate also depends on its fork's `util` crate and older
  cosmic-text/swash versions. Copying just the Android crate would not resolve this difference.
- The inspected `gpui-mobile` platform credentials implementation stores values in an in-memory map. Its
  platform clipboard implementation also uses process-local storage. These are not persistent Android services.
- The inspected Zedra backend returns an error from `write_credentials` and no value from `read_credentials`.
  OptChat needs its own Keystore-backed `SecureStore` through `optchat_platform::Platform` either way.

These are source-review findings, **not compiler diagnostics or device results**. Community screenshots,
an example that runs elsewhere, and a Play listing do not demonstrate compatibility with OptChat's pin.

## Local device and tools

ADB authorization was accepted by the owner. Read-only commands on the connected OnePlus reported:

| Property | Observed value |
|---|---|
| Device | OnePlus 9 5G, model LE2115 |
| Android | 14, API 34 |
| ABIs | arm64-v8a, armeabi-v7a, armeabi |
| Screen | 1080 x 2400; reported 60 Hz active mode, 60/120 Hz supported |
| Memory page size | 4096 bytes |
| Graphics features | Vulkan level 1; Vulkan version code 4198400 (1.1); OpenGL ES AEP |
| Other features | Camera and microphone advertised by the package manager |

Installed locally under `%LOCALAPPDATA%\OptChat\AndroidTools`:

- Android Platform Tools 37.0.1 (`adb` 1.0.41).
- Android Command-line Tools 22.0; the downloaded archive matched Google's published SHA256.
- Eclipse Temurin JDK 21.0.12.1+1; the downloaded archive matched the checksum from Adoptium's API.
- SDK platform 36 revision 2, build tools 36.0.0 and NDK 29.0.14206865; downloaded archives matched the
  checksums in Google's SDK repository metadata.
- Gradle 8.13, verified against the published SHA256; the wrapper pins this checksum.
- Rust's `aarch64-linux-android` standard library and rustfmt for toolchain 1.98.1.

`sdk/` is the SDK root; `jdk/jdk-21.0.12.1+1/` is the Java home within that directory.
The installation does not change the machine's global PATH. The build script sets process-local tool locations.
Installed Rust is 1.98.1; the Android Gradle Plugin is pinned to 8.13.0.
Run [doctor.ps1](doctor.ps1) to check current local state; its JSON output stays in ignored `build/`.

## Implementation follow-up

The owner selected GPUI and authorized the app port after the initial experiment. On 2026-10-15, the ARM64
hello window rendered on the OnePlus through Vulkan / Adreno 660; raw touch, cancellation and three Home/resume
cycles passed. That experiment has now been retired and replaced by the shared OptChat client and UI.

The [Android shell](app/src/lib.rs) installs Keystore-backed token storage, the shared Tokio client and the
existing GPUI phone layout. A NativeActivity subclass supplies Android's IME, clipboard, URL opening, system
insets and microphone permission. IME snapshots are queued across threads and applied through GPUI's focused
input handler, with UTF-16 selection/composition and field/session checks. Android networking uses rustls;
the existing native TLS configuration remains selected on other platforms.

The pinned `vendor/gpui-mobile` submodule and `patches/gpui-mobile/android-app.patch` preserve source provenance.
The patch continues to deliver native touch contacts, skip unchanged presentations and bound idle polling.
It also loads Bengali font variants and the device's compatible legacy bitmap emoji font when available.

See the [device qualification record](QUALIFICATION.md) for actual app results and remaining limitations.
The original research above records the candidates inspected at the time; it is not a claim that every
community backend is supported or that the Android client has production qualification.

## Packaging after the implementation choice

The user-facing download should be a signed APK that Android's installer can open. An AAB is an optional
Play upload artifact, not a file for users to install directly. Maintain a stable application ID, increasing
version codes and an owner-controlled signing identity, with private keys outside Git. Keep development
packages distinct from release packages. Android's [signing guide](https://developer.android.com/studio/publish/app-signing)
describes the APK and Play signing paths.

Then add pinned build tooling, CI, package/signature verification, checksums and a release manifest; qualify
install, update with saved settings, removal, reconnect and secure token restoration. Use the fake engine
before a dedicated real test workspace. GPUI's native libraries also need [16 KB page-size verification](https://developer.android.com/guide/practices/page-sizes);
this OnePlus's 4 KB configuration cannot establish that compatibility. Carry forward the project's existing
deferred license/notice status in release documentation. No production release approval is implied.
