# Windows release qualification

Updated 2026-10-16. **Production approval is pending.**

## Current x64 candidate

- Installer: `releases/windows/x64/OptChat-0.1.0.4-windows-x64-Setup.exe` (8,077,536 bytes).
- SHA256: `c495c2fd1372093e7df68b44a30e36aee156a4f8c36c06005eb99c88965e6f0f`.
- Source: `ae0c06b9f2f7988974cc5b9ba6e3328af6fa3823` plus uncommitted packaging/UI changes.
- Generated provenance: `releases/windows/x64/OptChat-0.1.0.4-windows-x64-release.json`.
- Signing: `CN=OptChat Development`, thumbprint `BBF7C8723A53DA06B05C7B1E1D9F9A276AACA68A`, no timestamp.
  Verification passes with local development trust; this is not public publisher signing.
- Build: Rust 1.98.1, VS Build Tools 17.14.41, Windows SDK 10.0.26100, Inno Setup 6.7.3.
- Machine: Windows 11 Pro 10.0.26200 (26200.9457), x64, AMD Radeon Graphics driver 31.0.21923.13006.

## ARM64 CI candidate

- [Installer](../../../releases/windows/arm64/OptChat-0.1.0.1-windows-arm64-Setup.exe): 7,683,427 bytes.
- SHA256: `ba42d2c07329c96fd97a7ce83e528bc7beca63b93c461214aa8ed78b3d7456ed`.
- Source: clean commit `8ee568eda3ceaa0ffda3dd71683c9041705b97a1`, native `aarch64-pc-windows-msvc`.
- Signing: unsigned development build; production publisher signing remains pending.
- [Original build-report data](../../../releases/windows/arm64/OptChat-0.1.0.1-windows-arm64-release.json)
  and the optional portable ZIP/checksum are kept alongside the installer.
- [Native CI run](https://github.com/fabihatasneem/optchat-client/actions/runs/38893377242) passed both x64 and
  ARM64 jobs, including shell unit tests, package inspection, install/upgrade/downgrade rejection, same-version
  repair and uninstall. The upgrade fixture covers revisions 0 to 1 from the same source.
- The downloaded `windows-arm64` archive matched GitHub's digest
  `3cb00e7ffdeb007df0225b4e959174324c0559f89663619c289dab8edcf80a60`.
  Installer/ZIP hashes matched the report, and the portable app's PE machine field was ARM64 (`0xaa64`).

These CI checks do not establish native graphical input, real-engine use, production signing or the complete
Windows desktop matrix. The earlier x64 desktop evidence below applies to its recorded candidate.

## Completed x64 desktop checks

| Check | Evidence |
| --- | --- |
| Native release build | 0.1.0.3 and 0.1.0.4 built with static CRT and checked DLL imports. |
| Native shell tests | 2 passed: settings-path isolation and Credential Manager Unicode round-trip/deletion/limits. |
| Per-user installation | 0.1.0.3 installed in a fresh test directory with Start shortcut and uninstall registration. |
| Upgrade | 0.1.0.3 to 0.1.0.4; resource/package versions agree, preferences retained. |
| Payload cleanup | Upgrade removed obsolete imports.txt and INSTALL.txt. |
| Downgrade | 0.1.0.3 rejected over 0.1.0.4 without changing the installed version. |
| Repair | Same-version reinstall passed and retained the settings canary. |
| Signatures | Setup, installed application and uninstaller verified with the local development certificate. |
| Installed app smoke | All 36 checks in tests/windows.ts passed from the installed location. |
| Uninstall | App, shortcut and registration removed; settings canary retained, then cleaned up by the test. |
| Packaging fixtures | Both PE architectures, layouts, manifest escaping/OS minimums, checksum corruption and wrong-architecture rejection. |
| Release guards | Dirty/untracked source and absent release tags rejected; generated hashes match actual output bytes. |
| Vendor preparation | CRLF source, idempotence and preservation of additional edits passed. |
| Workflow validation | actionlint 1.7.12, YAML parsing and embedded PowerShell syntax passed. |

Logs: `build/windows/recommendation-build.log`, `build/windows/recommendation-lifecycle.log`, and
`build/windows/installer-test-05023dc1-b484-4dc6-8021-8dba329aac17/`. These are local ignored evidence.
The isolated installer test did not replace the existing MSIX installation.

The smoke suite exercises app handlers/GPUI dispatch with a fake engine: first-run token setup, two workspaces
over one socket, rendering probes, Unicode/multiline input, Ctrl shortcuts, notices, reconnect, exit and relaunch.
It uses temporary file-backed test tokens. It does not establish native mouse, clipboard, IME, visual, microphone,
real-engine or credential persistence across upgrades. The Download updates button has not been visually qualified.

Earlier 0.1.0.1/0.1.0.2 x64 ZIP/MSIX builds passed MakeAppx validation, inspection and development-signature
checks; 0.1.0.1 MSIX activated successfully. This does not qualify the current optional MSIX output or migration
between installation formats. The superseded checked-in 0.1.0.2 files are retained locally under
`build/windows/superseded-0.1.0.2/`.

## Required before publishing

- [ ] Commit and tag the exact source, then build the production candidate in CI.
- [ ] Configure the protected release environment and native signing runners; run the changed workflow.
- [ ] Provide trusted organization signing with RFC 3161 timestamps for the app, setup and uninstaller.
- [ ] Supply approved project and dependency notices. Review is deferred at the owner's request.
- [ ] Test clean Windows 10 22H2 x64, current Windows 11 x64 and Windows 11 ARM64 standard-user profiles.
- [ ] Verify startup without developer tools, Start/taskbar icons, shortcuts, last-window exit and relaunch.
- [ ] Test upgrade from the previous public version with its saved data, an in-use upgrade, uninstall/reinstall,
  and Credential Manager retention using isolated credentials. CI's lower-version fixture uses the same source
  as the candidate; it tests installation mechanics, not historical data migrations.
- [ ] Exercise a real engine, HTTPS/WSS, invalid certificates/hostnames, proxies, corporate trust, token
  restoration, reconnection and workspace switching.
- [ ] Inspect fonts/icons, scrolling, large histories, multiple monitors and 100/150/200% DPI. Test native input,
  IME, clipboard text/images, attachments, dialogs and microphone capture/stop/permission denial.
- [ ] Verify Defender/SmartScreen behavior with the actual publisher signature and download URL.
- [ ] Review release notes and hashes, publish the approved draft, and verify download/manual update links.

Setup.exe is the initial public format. ZIP/MSIX/bundles are opt-in and need separate qualification before being
offered. Qualify managed deployment and MSIX-to-EXE migration separately. Server, S mode and 32-bit Windows are
outside initial support. Updates are manual; App settings opens release downloads.

See [BUILDING.md](../BUILDING.md) for release configuration. Local checks do not replace the outstanding
platform qualification and release approval.
