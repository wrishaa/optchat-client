# Platforms

| Platform | Shell | Status | GPUI backend at our pin |
|---|---|---|---|
| macOS | `shells/mac` | ships (dev bundle, ad hoc signed; notarization to do) | `gpui_macos` (Metal) |
| iOS / iPadOS | `shells/ios` | TestFlight builds | `gpui_ios` (zed PR #63068) + `patches/zed` |
| Linux | `shells/linux` | native shell, AppImage, x86_64/ARM64 development DEB/RPM packages | `gpui_linux` (Wayland + X11) on `gpui_wgpu` (Vulkan/GLES) |
| Windows | `shells/windows` | native shell + MSIX/ZIP build files; Windows qualification pending | `gpui_windows` (DirectX, DirectWrite) |
| Android | `shells/android` | [Shared-client development APK on OnePlus 9 / Android 14](../shells/android/QUALIFICATION.md) | Patched `gpui-mobile` over the existing pin; no Android backend inside pinned Zed |

## What a shell provides (`optchat_platform::Platform`)

| Hook | macOS | iOS | Linux | Windows | Android |
|---|---|---|---|---|---|
| `secure_store` (client tokens) | Keychain | Keychain | Secret Service (oo7) | Credential Manager | Keystore (JNI) |
| `resident_mb` (perf probes) | `proc_pidinfo` | `task_info` footprint | `/proc/self/statm` | `GetProcessMemoryInfo` | not implemented |
| `can_scan_qr` / `scan_qr` | test hook only | AVCaptureSession | test hook only | no | no |

Covered by GPUI itself (no hook needed): clipboard, opening URLs, windows, IME, credentials API (an alternative
token store).

**Not behind the trait yet** (candidates for the next hooks):
- **Dictation capture:** cpal in `crates/client/src/mic.rs`. It works on macOS, iOS, Linux (ALSA) and Windows
  (WASAPI); macOS has the existing mic-permission check; Android requests access through `Platform::prepare_microphone`.
- **Prefs directory:** chosen by each shell's `main.rs`.
- **Local engine token path:** `$HOME/.reframe/data/workspaces/local-client-token`, desktop only.
- **Headless test input:** AppKit-only, in `crates/ui/src/testcmd.rs`, with no-op stubs elsewhere.
- **Haptics and system notifications:** later.

## Linux system packages

These come from zed's `script/linux` at the pin. Debian/Ubuntu: `libasound2-dev libfontconfig-dev libssl-dev
libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev`, plus clang, cmake, pkg-config and a Vulkan driver. Fedora
equivalents: `alsa-lib-devel fontconfig-devel wayland-devel libxcb-devel libxkbcommon-x11-devel openssl-devel`.

## Windows toolchain

You need MSVC build tools (with the Spectre-mitigated libs), the Windows SDK ≥ 10.0.20348 and CMake. See
`vendor/zed/docs/src/development/windows.md`. Windows build scripts target `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc`, with static CRT linkage, app resources and MSIX packaging. Use PowerShell 7.4+;
`tools/zed-vendor.ps1` applies the same pinned GPUI patches without requiring Bash. The native shell stores
settings in `%APPDATA%\OptChat`, uses Credential Manager for tokens and exits when its last window closes.
Windows 10 22H2/Windows 11 x64 and Windows 11 ARM64 are intended qualification targets; 32-bit, Windows 7/8,
Windows Server, S mode and a Store listing are outside the initial scope. These are targets, not verified support.
See [the Windows guide](../shells/windows/README.md) and
[qualification checklist](../shells/windows/release/QUALIFICATION.md).

## Linux checks

`cargo run -p optchat-linux` launches the native desktop app. `shells/linux/demo.sh` launches a temporary fake-engine
demo. `cargo build -p optchat-linux && deno run -A tests/linux.ts` verifies native window rendering, connections,
keyboard input, workspace switching and reconnect using isolated test tokens and preferences.
Shared bindings use GPUI's `secondary-` modifier (Command on macOS, Ctrl elsewhere); Linux navigation overrides
live in the shell. `shells/linux/release/bundle.sh --check` builds an AppImage and verifies the packaged executable
before moving it to `dist-dev/` alongside its checksum and build information. Packaging tools and the AppImage
runtime are pinned and verified by size/SHA256. The current package is tested on Ubuntu 26.04 x86_64 and references
GLIBC 2.43. `shells/linux/tests/test-package.sh` passes on Wayland and the X11 backend through XWayland; compatibility
with other distributions/older releases and standalone Xorg desktops is not yet verified. Camera scanning and the
full mouse-driven UI suite remain future work. Flatpak is outside the chosen release scope.

## Managed Linux distribution

`shells/linux/release/container-build.sh` selects the agreed native baseline: Ubuntu 24.04 for DEBs, Fedora 43 for RPMs.
Both x86_64 and ARM64 are in scope. Qualify DEBs on Ubuntu 24.04/26.04 and Debian 13; qualify RPMs on Fedora 43/44.
Separate native Cargo targets prevent reuse of newer-host binaries. `package_deb.py` infers ABI dependencies using
dpkg-shlibdeps, while RPM uses its native ELF dependency generator. `check-install.sh` covers container lifecycle checks;
`test-native-package.sh` covers extracted-app graphical smoke tests. Desktop qualification is still separate.
`apt_repository.py` and `rpm_repository.py` prepare signed APT/DNF repositories with an owner-supplied GPG key.
Fresh Ubuntu 24.04/x86_64 and Fedora 43/x86_64 development packages require GLIBC 2.39 and pass extracted-app
smoke checks on Ubuntu 26.04 with Wayland and XWayland. The earlier host development `.deb` and AppImage
require GLIBC 2.43. Rootless container lifecycle qualification is recorded separately under `build/qualification/`.
The [2026-10-14 native CI run](https://github.com/fabihatasneem/optchat-client/actions/runs/38557757484) passed all
four jobs for source commit `482af41980829b8468b636d10af71eae7a00bfef`: DEB/RPM builds on x86_64 and native
ARM64, including dependencies, install, upgrade, removal and reinstall across the five agreed distributions on
each architecture. Both RPM jobs also passed 24 HTTPS/WSS checks per Fedora version (43 and 44), covering
DEFAULT/FUTURE policies, stricter protocol limits, certificate/hostname rejection and preservation of certificate
verification across system-policy configuration. Development packages, checksums and build/qualification evidence
are available in that run's artifacts; GitHub retains these artifacts for 14 days.

The approved package maintainer is `Fabiha Tasneem <fabiha@reframe.so>`. Copyright/license work is deferred.
Installed-app isolated checks on this Ubuntu 26.04 GNOME computer passed for desktop-entry launch, the Add engine
form, actual Secret Service storage and token restoration after relaunch, Unicode/multiline input, XWayland
clipboard, image processing, large-history rendering/scrolling, reconnect, real microphone capture/stop and clean
exit. Wayland and XWayland were exercised; final `0local.4` packages passed 23 smoke checks on each backend.
Native input automation was not delivered by GNOME. Wayland clipboard with real input, normal file-picker/portal
use, logout/reboot persistence, the clean-desktop matrix and ARM64 desktop hardware remain manual qualification.
Real-engine access is deferred by the owner. These results do not establish customer desktop support yet.

Final license/notice metadata, clean-desktop and real-engine qualification, production keys and HTTPS hosting must
be complete before a stable release. Flatpak is excluded from the chosen scope.
See `shells/linux/BUILDING.md` and `release/QUALIFICATION.md` for the ordered workflow and sign-off record.
