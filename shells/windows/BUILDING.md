# Build and release OptChat for Windows

See the [installation guide](README.md) for application setup and the
[qualification record](release/QUALIFICATION.md) for platform validation and release status.

## Package formats and architectures

The default package is a per-user **Setup.exe**. Portable **ZIP** and managed **MSIX** packages are opt-in.
Build a native package for each CPU architecture:

- **x64**, `x86_64-pc-windows-msvc`: Intel/AMD 64-bit PCs. Intended minimum: Windows 10 22H2 (build 19045).
  Also qualify current Windows 11 x64 systems. These are qualification targets, not verified support.
- **ARM64**, `aarch64-pc-windows-msvc`: Windows on ARM, including Snapdragon PCs. Intended minimum:
  Windows 11 (build 22000). Prefer the native package over x64 emulation.
- **32-bit x86/ARM32**: excluded from the initial release. Supporting them needs separate GPUI, dependency,
  compiler and desktop qualification; changing an installer label does not make a 32-bit application.
- **Windows 7/8, Server, S mode and a Microsoft Store listing**: outside the initial scope.

Home, Pro and Enterprise do not need separate binaries. Choose by CPU and OS version, then qualify the editions
and policies you intend to support. WSL is a Linux environment; use native Windows tools for these builds.
A GPU/driver compatible with the pinned GPUI DirectX backend is required.

`packaging/OptChat.iss` and `release/build-installer.ps1` produce an offline, per-user EXE installer using
Inno Setup. It bundles the application and notices, creates a Start shortcut and registers an uninstaller.
An optional MSIX bundle lets Windows choose the CPU architecture.
The App settings page links to release downloads. Updates are manual: rerun a newer installer;
MSIX updates require a newer version with the same identity/publisher.
The installer blocks downgrades and supports same-version repair. It does not migrate an MSIX installation;
remove the old installation in Windows Settings before switching distribution channels.

## Build files and output

`src/` provides native startup, Windows shortcuts, last-window exit, `%APPDATA%\OptChat\prefs.json`, Credential
Manager tokens and process memory measurement. `OPTCHAT_PREFS_DIR` isolates test settings. `build.rs` and
`packaging/` provide executable resources, the MSIX manifest and icons. `../../tools/zed-vendor.ps1` applies the same
pinned GPUI patches as the Bash helper without requiring Bash or discarding local submodule edits.

`release/build.ps1` compiles and packages; `bundle.ps1` combines matching x64/ARM64 MSIX files;
`new-test-certificate.ps1` creates a development signing identity. Under `tests/`, `check-package.ps1` inspects packages,
`test-packaging.ps1` runs cross-platform parser and package fixture checks, and `test-vendor.ps1` checks CRLF
preparation, idempotence and edit preservation. The Windows GitHub Actions workflow builds
both architectures on native runners and retains unsigned development artifacts for 14 days.

For workspace version `0.1.0` and revision `0`, the x64 output is:

```text
dist-dev/windows-x64/
  OptChat-0.1.0.0-windows-x64-Setup.exe
  OptChat-0.1.0.0-windows-x64-Setup.exe.sha256
  OptChat-0.1.0.0-windows-x64-release.json
```

ARM64 filenames use `arm64`. The generated release manifest records the source, toolchain, package version,
artifact sizes/hashes and signature status. It replaces separate build-info and installer-info sidecars.
The installed application contains `optchat.exe`, notices, a small `build-info.json` for diagnostics, and the
uninstaller. Icons/fonts are embedded in the executable. `imports.txt` remains under `build/windows/` as CI evidence.
The internal ZIP used to assemble Setup.exe also stays under `build/windows/` unless `-Formats zip` is selected.
MSIX adds the manifest and visual assets; unsigned MSIX filenames end in `-unsigned.msix`.
Ship each public package with its checksum. Development `.cer` files belong only on test machines.

The repository links the MSVC runtime statically. The build checks the PE architecture and rejects unexpected
direct dynamic VC++ runtime imports. Windows system DLLs, Schannel TLS and WASAPI audio remain OS services.
No Electron, Node, WebView2 or OpenSSL runtime is bundled. Validate `imports.txt` and clean-machine startup for
each final artifact; static configuration alone is not evidence of runtime completeness.

## Source-build prerequisites

1. Clone the desired source branch into a short native path such as `C:\src\optchat-client`. Existing clones can
   fetch and switch to the desired branch. The build initializes the pinned submodule.
2. Install **PowerShell 7.4+**, **Git for Windows**, **Rust via rustup**, **CMake** and **Visual Studio 2022 or newer**
   with **Desktop development with C++**. The repository selects Rust 1.98.1 in `rust-toolchain.toml`.
3. Install matching MSVC tools and Spectre-mitigated libraries: x64/x86 for x64; ARM64 for ARM64. Install a
   **Windows SDK >= 10.0.20348**, including `rc.exe`, `fxc.exe`, `makeappx.exe` and `signtool.exe`.
   SDK 10.0.26100 is suitable. Import `shells/windows/windows.vsconfig` through Visual Studio Installer for a
   VS 2022 component checklist covering both targets. No PostgreSQL or full Zed application setup is needed.
4. Use the matching Visual Studio developer shell. Start `pwsh -NoProfile` inside it to inherit the C++ environment
   while running PowerShell 7. Alternatively initialize VS directly from PowerShell 7:

```powershell
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -property installationPath
& "$vs\Common7\Tools\Launch-VsDevShell.ps1" -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
# On native ARM64, use -Arch arm64 -HostArch arm64 instead.
```

Check `cl.exe`, `link.exe`, `dumpbin.exe`, `cmake`, `cargo` and `git` are on PATH. Scripts locate the SDK tools.
Unset custom `RUSTFLAGS`, encoded flags and target-specific flag environment variables, which override the static
CRT configuration. Prefer native builds for each architecture. Cross-compiling ARM64 needs matching VS cross tools
and still requires ARM64 hardware qualification.

If `git apply --check` fails in an existing Windows clone, first check whether `patches/zed/*.patch` retained
CRLF line endings from before `.gitattributes` was added. Preserve any patch edits and restore clean patch files
with LF line endings before rerunning the helper. Do not reset the vendor checkout to hide unrelated local edits.

## Build and inspect

EXE packaging requires [Inno Setup 6.7.3](https://jrsoftware.org/isdl.php).
The template enforces this version; CI verifies the compiler installer's checksum and publisher.
To wrap an existing portable ZIP without recompiling:

```powershell
./shells/windows/release/build-installer.ps1 `
  -PortablePackage ./dist-dev/windows-x64/OptChat-0.1.0.0-windows-x64-portable.zip -Architecture x64
# If ISCC.exe is not discoverable, pass -Compiler 'C:\path\to\ISCC.exe'.
# For build.ps1, set OPTCHAT_INNO_COMPILER to that exact compiler path when multiple versions are installed.
```

This wraps the existing payload without rebuilding Rust, verifies its checksum and architecture, and writes
`OptChat-0.1.0.0-windows-x64-Setup.exe`, `.sha256` and `-release.json` beside the ZIP (or in `-OutputDirectory`). For a signed payload, supply its matching
`-CertificateThumbprint` and, for production, `-TimestampUrl`. Both setup and its embedded uninstaller are signed.
The release payload must already carry approved notices. Production signing does not replace qualification.

`build.ps1` builds EXE by default. Select `-Formats exe,zip,msix` to exercise all optional formats.
To validate installation, upgrade, downgrade rejection, same-version repair and removal using two versions:

```powershell
./shells/windows/tests/test-installer.ps1 `
  -Installer ./dist-dev/windows-x64/OptChat-0.1.0.3-windows-x64-Setup.exe `
  -Upgrade ./dist-dev/windows-x64/OptChat-0.1.0.4-windows-x64-Setup.exe `
  -Architecture x64 -RequireSignature -SmokeTest
```

The test refuses to replace an existing EXE installation or Start menu folder. It uses a unique install directory,
checks shortcuts and uninstall registration, verifies settings retention and removes the test installation.
`-SmokeTest` requires Deno and runs the installed application against an isolated fake engine.
Logs remain under `build/windows/installer-test-*`. Run in-use upgrades, real-engine and visual checks separately.

From the repository root in developer PowerShell 7:

```powershell
./shells/windows/tests/test-packaging.ps1
./shells/windows/tests/test-vendor.ps1
./shells/windows/release/build.ps1 -Architecture x64 -Formats exe,zip,msix
./shells/windows/tests/check-package.ps1 `
  -Package ./dist-dev/windows-x64/OptChat-0.1.0.0-windows-x64-portable.zip -Architecture x64
./shells/windows/tests/check-package.ps1 `
  -Package ./dist-dev/windows-x64/OptChat-0.1.0.0-windows-x64-unsigned.msix -Architecture x64
```

On ARM64 substitute `arm64` in arguments/paths. `-Formats zip` builds just the portable package. Release EXEs use
the Windows GUI subsystem so launch does not open a console. Compilation uses `--locked --release` with a separate
Windows target directory. The script never installs packages or changes certificate trust.
Existing artifacts are not overwritten: increment `-Revision` or move old artifacts aside. Versions are
`workspace-major.minor.patch.Revision`; Store-specific versioning/identity would need a submission review.

Run native shell tests after building (use the corresponding target on ARM64):

```powershell
cargo test --locked --release -p optchat-windows --target x86_64-pc-windows-msvc --target-dir target/windows
```

The Credential Manager test creates and deletes its own unique entry. Tests do not replace installed-app checks.

For an isolated native app smoke test, install Deno on the **test machine only** and run:

```powershell
$env:OPTCHAT_TEST_BINARY = 'C:\path\to\extracted\OptChat\optchat.exe'
deno run -A tests/windows.ts
```

This copies the executable to a temporary Unicode/spaced path, starts it without developer tools on PATH,
uses an isolated fake engine and file-backed test tokens, and exercises the app's own test hooks. It does not
use real credentials or real workspaces and does not substitute for visual, mouse, IME, audio or secure-store QA.

## Development signing and installation

Unsigned MSIX files cannot be installed normally. Create a development certificate **once**, then reuse it for upgrades:

```powershell
$thumb = ./shells/windows/release/new-test-certificate.ps1
./shells/windows/release/build.ps1 -Architecture x64 -Revision 1 -CertificateThumbprint $thumb
```

The private key stays non-exportable in your current user's Personal store. Only the public
`build/windows/test-signing/OptChat-Development.cer` is exported. The script does not automatically trust it.
Review and import that public certificate on the test machine from an **elevated** PowerShell window:

```powershell
Import-Certificate -FilePath ./build/windows/test-signing/OptChat-Development.cer `
  -CertStoreLocation Cert:\LocalMachine\TrustedPeople
```

Then run the Appx deployment cmdlets in **Windows PowerShell 5.1**, as the normal signed-in user:

```powershell
Add-AppxPackage -Path ./dist-dev/windows-x64/OptChat-0.1.0.1-windows-x64.msix
Get-AppxPackage -Name Reframe.OptChat | Select-Object Name, Version, Architecture, InstallLocation
```

Launch from Start. For ZIP testing, extract the entire folder and run `OptChat\optchat.exe`. Add `-RequireSignature`
to `check-package.ps1` after trusting the test certificate. Rebuild with `-Revision 2` and the same `$thumb` for an
upgrade, then install the newer MSIX. Remove the current-user package with:

```powershell
Get-AppxPackage -Name Reframe.OptChat | Remove-AppxPackage
```

Settings intentionally remain in `%APPDATA%\OptChat` across ZIP/MSIX and uninstall; the manifest disables AppData
write virtualization. Tokens remain in Credential Manager unless removed in the app. Verify this lifecycle on
the intended Windows versions. Never add broad AppData cleanup. Managed machine-wide installation can provision
a signed package using elevated `Add-AppxProvisionedPackage -Online -PackagePath <package.msix> -SkipLicense`;
qualify that separately. OptChat has no Store offline-license file.

Remove the development certificate's Trusted People entry when no longer needed. Copying the public `.cer` to a
second test machine allows package trust but does not let it sign upgrades with the original private key.

## Optional architecture bundle

Build both architectures at the same version and Publisher, then:

```powershell
./shells/windows/release/bundle.ps1 `
  -X64Package ./dist-dev/windows-x64/OptChat-0.1.0.0-windows-x64-unsigned.msix `
  -Arm64Package ./dist-dev/windows-arm64/OptChat-0.1.0.0-windows-arm64-unsigned.msix
```

Output is `dist-dev/windows-bundle/OptChat-0.1.0.0-windows-unsigned.msixbundle` plus a checksum. Pass a matching
certificate thumbprint to sign the bundle. The final bundle must be signed and trusted; individual development
input packages can be unsigned. Identity, publisher, version and architecture must match.

## CI and production

Pull requests and branch dispatches use `windows-2022` x64 and `windows-11-arm` native ARM64 runners. They build
unsigned development EXE/ZIP/MSIX artifacts, run package/shell tests, and validate EXE installation, upgrade,
downgrade rejection, repair and removal. They cannot access production signing credentials. These jobs do not
run desktop smoke tests. A Windows Server build runner does not prove Windows 10 compatibility.

Generated `dist/`, `dist-dev/` and `build/` directories are ignored by Git. Selected installers checked in for
review live under `releases/windows/<architecture>/`, with SHA256 and one generated release manifest. Preserve versioned
filenames and retest before replacing an artifact. Signing and qualification status are recorded in provenance
and [QUALIFICATION.md](release/QUALIFICATION.md).

### Production workflow setup

The same workflow accepts tags `windows-v<workspace version>.<revision>`; revision must be 1..65535. It is
disabled until repository administrators configure the following. No release has been published by this change.

1. Create a protected GitHub environment named `windows-release`, restricted to the intended release tags,
   with required maintainer approval. Protect tag creation and review workflow changes before tagging.
2. Provide dedicated, disposable native Windows x64 and ARM64 signing runners with the standard `Windows` and
   `X64`/`ARM64` labels plus `optchat-release`. Use fresh user profiles, an interactive desktop/GPU for smoke
   tests, the documented build tools, Inno Setup 6.7.3, Deno 2.x, and no existing OptChat EXE installation.
   Never run untrusted pull requests on these runners.
3. Provision the organization's signing provider and accessible code-signing certificate in the runner user's
   Personal store. Keep keys in the organization's hardware/service-backed provider; never commit private keys.
   Configure these **environment variables in GitHub Settings > Environments**:

   | Variable | Value |
   | --- | --- |
   | `WINDOWS_PUBLISHER` | Exact certificate subject |
   | `WINDOWS_CERTIFICATE_THUMBPRINT` | Code-signing certificate thumbprint |
   | `WINDOWS_TIMESTAMP_URL` | HTTPS RFC 3161 timestamp service |
   | `WINDOWS_LICENSE_NOTICE` | Repository-relative path to approved project and dependency notices |
   | `WINDOWS_RELEASE_ENABLED` | `true`, only once the preceding requirements are met |

4. Commit the source and approved notice, then create the matching release tag on that commit. The build refuses
   dirty source, a mismatched tag, a self-signed leaf certificate, missing notices, invalid signatures or missing
   timestamps. It signs the application, setup and embedded uninstaller and writes to `dist/windows-<architecture>/`.
5. Each architecture builds a lower-version test fixture and the tagged candidate, then checks installation,
   upgrade, downgrade rejection, repair, signatures, metadata, settings retention, native smoke and uninstall.
   The baseline is generated from the same source to test the installer; it does not replace upgrade testing
   from the previous publicly released app and its data.
6. After both jobs pass, CI verifies the downloaded artifact hashes and source commit, then creates a **draft
   GitHub Release** with the two EXEs, checksums and generated manifests. Logs remain CI evidence. Existing
   published releases and assets are never overwritten. Merge workflow changes to the default branch before
   creating a release tag so GitHub's release-token permissions can resolve that source.
7. Complete the remaining qualification, add release notes, then publish the reviewed draft. Enable immutable
   releases in repository settings where available. Users reach these downloads through App settings and the
   Windows guide. Automatic update installation is not implemented.

For a manual production build, supply `-Release -Publisher <subject> -CertificateThumbprint <thumbprint>
-TimestampUrl <https-url> -LicenseNotice <approved-file>` from the clean tagged checkout. A guard can validate
the notice's presence, not its legal completeness. Project license and dependency redistribution review remain
deferred at the owner's request, so production builds remain gated on an approved notice.

GitHub Releases is the normal distribution location. This branch also retains a selected EXE in Git as requested:
commit source first, build from that commit, then add the generated EXE/checksum/manifest in a separate artifact
commit. Do not tag that artifact commit as the source of an already built binary. Current uncommitted candidates
are marked `source_dirty: true` and `release: false` in their generated manifests.

Complete [release/QUALIFICATION.md](release/QUALIFICATION.md) before public distribution, including clean-machine,
real-engine, TLS, GPU/input/audio, install/upgrade/removal, production signing, notices and HTTPS hosting.

References: [MSIX packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[signing](https://learn.microsoft.com/en-us/windows/msix/package/sign-app-package-using-signtool),
[bundling](https://learn.microsoft.com/en-us/windows/msix/packaging-tool/bundle-msix-packages),
[AppData virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization).

## Directory layout

Linux and Windows use the same platform-folder layout:

```text
shells/<platform>/
  README.md          Installation, engine setup, updates and removal
  BUILDING.md        Source builds, packaging and release procedure
  Cargo.toml         Native shell crate
  src/               Native startup and platform implementation
  packaging/         Package manifests and templates
    assets/          Package icons and logos
  release/           Build, signing and repository publishing helpers
    QUALIFICATION.md Release evidence and remaining checks
  tests/             Package inspection and lifecycle checks
```

Platform-specific build inputs and developer launch helpers remain beside the crate.
Shared application smoke suites remain in the repository's top-level `tests/` directory.
