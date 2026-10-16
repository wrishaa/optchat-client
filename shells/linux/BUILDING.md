# Build and release OptChat for Linux

This guide is for maintainers building, testing, signing and publishing Linux packages.
**To install OptChat on your own computer, start with the [installation guide](README.md).**

## Automated preview releases

The Linux workflow reads the version from `Cargo.toml`. Normal branch runs keep increasing development
package revisions automatically. A `linux-v<version>-preview.<revision>` tag selects an exact `0local.<revision>`
and its previous revision for upgrade checks; the tag's version must match Cargo. Choose a new revision greater
than the last distributed preview (currently 7). Retries of the same run/tag retain their versions.

After all four native DEB/RPM jobs and their lifecycle/Fedora TLS checks succeed, the tag workflow verifies
each installer's architecture, checksum and clean source commit, then creates a **draft prerelease** with
installers, build reports, installation guides and `linux-release.json`. It never publishes automatically or
overwrites an existing release/asset. Review desktop and real-engine qualification before publishing.
Version tags and pushes require explicit maintainer action. For example, the next unused preview revision
can be selected with `linux-v0.1.0-preview.8`; this example does not create or push a tag.

This automation distributes development previews. Production DEB/RPM metadata, signed APT/DNF repository
setup, key ownership and hosting still follow the production sections below; the deferred notice requirements
are not bypassed. The checked-in `releases/linux/` packages remain selected, verified snapshots rather than
outputs automatically committed back to Git.

The agreed minimum build system is **Ubuntu 24.04**. Build a DEB there and qualify that same package on Ubuntu 24.04,
Ubuntu 26.04 and Debian 13. Build the RPM separately on **Fedora 43** and qualify it on Fedora 43 and Fedora 44.
Both x86_64 and ARM64 are in scope. Flatpak is outside this release's scope.

The [Linux preview release](https://github.com/fabihatasneem/optchat-client/releases/tag/linux-v0.1.0-preview.7)
provides direct DEB/RPM downloads for both architectures, with SHA256 sidecars and a release provenance manifest.
Its packages are the verified `0local.7` outputs of [CI run 38689085276](https://github.com/fabihatasneem/optchat-client/actions/runs/38689085276),
built from source commit `78d8c1d1fef864b4e760b95fe4963b75afef659d`. They are unsigned development previews;
manual Linux desktop testing remains pending. The release tag identifies the package source commit.

Native DEB/RPM packages with signed package-manager repositories follow the current
[OpenAI Linux distribution model](https://learn.chatgpt.com/docs/linux/linux-app). ARM64 uses a native ARM64 builder:
an x86_64 host with Podman does not become an ARM64 builder. GitHub provides
[native `ubuntu-24.04-arm` runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
if that is the chosen build route. Desktop qualification still requires each architecture.

The earlier host development DEB and AppImage were built on Ubuntu 26.04 and need GLIBC 2.43. The first native
Ubuntu 24.04/x86_64 DEB and Fedora 43/x86_64 RPM now require GLIBC 2.39, with extracted-app fake-engine checks
passing on Ubuntu 26.04's Wayland and XWayland backends. These remain development artifacts. The helpers below
write baseline builds into separate directories and reject newer binaries with incompatible GLIBC requirements.
The x86_64 DEB development revisions `0local`/`0local.1` have passed container dependency, install, upgrade, removal/purge, data
preservation and reinstall checks on Ubuntu 24.04, Ubuntu 26.04 and Debian 13. Fedora 43 and Fedora 44 have passed
the equivalent RPM checks. Signed APT metadata and RPM/repomd rehearsals also passed using ephemeral test keys.
Native x86_64 and ARM64 builds and container lifecycle checks have passed in CI; see
[the recorded results](../../docs/platforms.md#managed-linux-distribution). The clean desktop matrix,
real engines and production signing/hosting remain separate steps.

Development revision `0local.2` refreshes both packages with the approved maintainer contact. Its package metadata,
changelogs and checksums have been verified; the application binaries match the previously tested `0local.1`
binaries. The full lifecycle records above cover `0local`/`0local.1`.

Revision `0local.3` added Fedora system-policy support and passed the full five-OS lifecycle matrix, upgrading
from `0local.2`. Revision `0local.4` additionally preserves certificate verification when reapplying system policy.
Both formats still require GLIBC 2.39.
Both `0local.4` packages passed the 23-check extracted-app smoke suite on each of the host's Wayland and
XWayland backends; their checksums and dependency lists were also verified.

On the Ubuntu 26.04/x86_64 GNOME development computer, the installed `0local.3` DEB passed isolated desktop
checks through its registered desktop entry: Add engine, real Secret Service token storage and restoration after
quit/relaunch, Unicode/multiline input, clipboard copy/paste under XWayland, image attachment processing, large
history rendering/scrolling, resize/full-screen commands, reconnect and real microphone capture/stop. Both
Wayland and XWayland were exercised. This computer has development tools, so it is not a clean-customer VM.
Native keyboard automation was not delivered by GNOME; keyboard bindings were checked through GPUI dispatch.
Wayland clipboard ownership still needs a real compositor input event and remains a manual check. The image
processing check bypassed the file picker, so normal portal/file-picker use also remains a manual check. Real
engine access was deferred by the owner; desktop logout/reboot has not been tested.

Copyright/license work is deferred; [the notice template](packaging/COPYRIGHT.template) will be completed later.
Development packaging remains available without it. Lintian's remaining error is the missing owner-approved
copyright notice. Rpmlint reports the development license
placeholder and a static OpenSSL crypto-policy warning. The local native-tls patch restores the complete OpenSSL
`system_default` policy after the upstream builder, preserving any stricter caller protocol bounds. The actual
client HTTPS/WSS transports passed 24 checks each on Fedora 43 and Fedora 44: allowed TLS 1.2/1.3 connections,
untrusted/expired/wrong-host certificate rejection, legacy TLS and policy-excluded cipher rejection, and a stricter
TLS 1.3 minimum plus FUTURE policy. A configuration requesting `VerifyMode=-Peer` still rejects untrusted
certificates. The static warning detects the upstream cipher-setting symbol even though
the system policy is subsequently restored; it has been reviewed against this runtime evidence.
The fake-engine desktop smoke suite uses loopback HTTP and does
not establish real-engine TLS, keyring, microphone or clean-desktop qualification.

## 1. Install the local builder

On your Ubuntu machine, from any directory:

```bash
sudo apt update
sudo apt install podman uidmap passt fuse-overlayfs
podman info --format '{{.Host.Security.Rootless}}'
```

The last command should print `true`. Run Podman as your regular user. Do not prefix the build commands with sudo.
If Podman reports missing subordinate UID/GID ranges, inspect `grep "^$(id -un):" /etc/subuid /etc/subgid` and
follow the [rootless setup documentation](https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md).
Do not invent ranges that overlap other users. Desktop VM tools are a separate setup step.

## 2. Build the x86_64 development packages

From your local repository root:

```bash
shells/linux/release/container-build.sh deb
shells/linux/release/container-build.sh rpm
```

These install the pinned Rust toolchain into isolated Ubuntu 24.04/Fedora 43 builder images, build from source with
Cargo.lock, and generate native dependency metadata. They mount the repository and a Cargo cache; they do not
forward your desktop, audio, keyring, SSH agent or home directory. Native targets are separated by distro and CPU
under `build/native/`, so the old Ubuntu 26.04 target directory is never reused.

For the current version, the outputs on this machine are:

```text
dist-dev/ubuntu24.04-x86_64/optchat_0.1.0-0local_amd64.deb
dist-dev/fedora43-x86_64/optchat-0.1.0-0local.fc43.x86_64.rpm
```

Each package has `.sha256` and `.build.json` sidecars. The directory also records the base and builder image
identities. Base images are refreshed from their versioned upstream tags; set `OPTCHAT_BUILD_REFRESH=1` to bypass
image-layer cache and refresh installed builder packages for a release. These are repeatable build recipes and
provenance records, not a promise of bit-for-bit reproducibility. Package builds may take several minutes.

## 3. Build ARM64 natively

On an ARM64 Linux machine or native ARM64 runner with rootless Podman, use the same two commands from step 2.
The output directories become `ubuntu24.04-aarch64` and `fedora43-aarch64`; Debian names the package architecture
`arm64`, while RPM names it `aarch64`:

```text
dist-dev/ubuntu24.04-aarch64/optchat_0.1.0-0local_arm64.deb
dist-dev/fedora43-aarch64/optchat-0.1.0-0local.fc43.aarch64.rpm
```

Do not reuse an x86_64 binary or infer ARM64 support from a cross-compiled file. Record the actual native builder
and perform ARM64 desktop qualification before publishing ARM64 support.

The [Linux development packages workflow](../../.github/workflows/linux-packages.yml)
uses GitHub's native `ubuntu-24.04` and `ubuntu-24.04-arm` runners with these same baseline containers. GitHub
[requires a manually dispatched workflow to exist on the default branch](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow).
Matching source pushes trigger automatic builds. For manual runs, after the workflow is reviewed
and merged to the default branch, open
**Actions → Linux development packages → Run workflow**, and select the release branch. Each native DEB/RPM job
builds an older and newer development revision and checks package lifecycle including upgrade. CI uses
`0local.<2 × run number>` and `0local.<2 × run number + 1>`, so a later workflow run has a higher package version;
rerunning the same workflow run keeps its version numbers. After successful checks it uploads
a user download named `linux-deb-<CPU>` or `linux-rpm-<CPU>`: the newer installer, its checksum, an `INSTALL.md`
with exact installation commands, and a `build-info/` folder. Separate `linux-evidence-<format>-<CPU>` artifacts
contain both revisions, build reports and qualification logs for maintainers. The workflow does not
sign packages or publish a release. Both RPM jobs also run the HTTPS/WSS policy checks on Fedora 43 and 44.
Downloaded packages still require the desktop qualification in step 5.

`prepare_download.py` assembles the user download and verifies that the package, checksum and build report agree.
For example, after creating a development DEB:

```bash
python3 shells/linux/release/prepare_download.py \
  dist-dev/ubuntu24.04-x86_64/optchat_0.1.0-0local_amd64.deb \
  --output-dir build/downloads/deb-x86_64
```

Use a new or empty output directory. Its contents can be zipped and shared together. Existing older CI archives
contain `dist-dev/` and `build/` instead; the installation guide explains how to find the installer in those archives.

Run the same TLS qualification locally after building the baseline RPM:

```bash
shells/linux/tests/check-tls.sh
```

It builds the transport-only example from the actual client crate, creates temporary certificates and loopback
servers, and records results under `build/qualification/`. Test trust roots and stricter OpenSSL configurations
apply only to disposable client processes; this does not change the host's trust store or crypto policy.

## 4. Test dependencies and package lifecycle

On each architecture, run these against its matching packages:

```bash
shells/linux/tests/check-install.sh dist-dev/ubuntu24.04-x86_64/optchat_0.1.0-0local_amd64.deb
shells/linux/tests/check-install.sh dist-dev/fedora43-x86_64/optchat-0.1.0-0local.fc43.x86_64.rpm
```

Replace paths with the ARM64 paths on that machine. The DEB script tests Ubuntu 24.04, Ubuntu 26.04 and Debian 13;
the RPM script tests Fedora 43 and 44. It uses disposable rootless containers to test native architecture, dependency
resolution, install, executable layout/mode, direct shared-library resolution, removal/purge, preserved user-data
sentinel and reinstall. Each run saves verified package snapshots, a SHA256 manifest, logs and OS image identities
under `build/qualification/`, so all target systems test the same bytes even if a package is rebuilt concurrently.
Public repository metadata and downloaded dependencies are cached per OS/architecture under
`build/qualification-cache/`. Every check still starts in a fresh container and resolves packages through the
distribution's signed repositories. Test containers retain OptChat documentation that minimal OCI images normally omit.

To test an initial upgrade, rebuild with `container-build.sh deb --revision 0local.1` (and likewise `rpm`), then
pass the newer package as the first argument and the original `0local` package as the second argument to
`check-install.sh`. If no previous package is supplied, upgrade is explicitly untested.

These containers have no desktop; they cannot qualify rendering, portals, audio or secure token storage.

## 5. Qualify the actual desktop packages

Create clean desktop VMs/hardware for each target OS and architecture. They should not have Rust, compiler headers
or your development configuration. Copy the package and checksum, then install through the distro package manager:

```bash
# Ubuntu/Debian, in the directory containing the downloaded package:
sha256sum -c optchat_0.1.0-0local_amd64.deb.sha256
sudo apt install ./optchat_0.1.0-0local_amd64.deb
optchat

# Fedora:
sha256sum -c optchat-0.1.0-0local.fc43.x86_64.rpm.sha256
sudo dnf install ./optchat-0.1.0-0local.fc43.x86_64.rpm
optchat
```

Use the correct ARM64 filenames there. Also launch from the application menu, test clipboard/files/microphone,
and connect through the normal UI to a real test engine. Confirm secure token persistence after reboot, upgrade,
remove/reinstall, and behavior with a locked/unavailable keyring. Test the supported Wayland/X11 paths explicitly.

For fake-engine checks, copy/clone the test source and install Deno 2.x (no Rust build is needed), then run:

```bash
shells/linux/tests/test-native-package.sh /absolute/path/to/optchat.deb
# or an RPM; extraction uses rpm2cpio/cpio or the existing native Fedora builder image
shells/linux/tests/test-native-package.sh /absolute/path/to/optchat.rpm
```

The suite extracts into temporary files and tests the packaged executable with isolated tokens/preferences; it does
not exercise the installed launcher or real Secret Service. Installed-menu and normal-UI tests remain necessary.
Use [QUALIFICATION.md](release/QUALIFICATION.md) to keep ten OS/architecture sign-off records. Run Lintian on the DEB
and Rpmlint on the RPM and review all findings before stable release.

## 6. Supply real release metadata and rebuild candidates

The approved default maintainer is `Fabiha Tasneem <fabiha@reframe.so>` for both package formats and their changelogs.
Override it with `--maintainer` or `OPTCHAT_PACKAGE_MAINTAINER` when needed; the command-line option takes precedence.

Copyright/license work is deferred for now. Continue with development revisions (`0local` or `0local.N`) while
[the notice template](packaging/COPYRIGHT.template) is completed later. The template is a drafting aid and is
not automatically included in packages. Production revisions still need the project's approved notice, and RPM
also needs its approved SPDX license expression. Do not choose a new project license merely to fill packaging fields.
Include applicable third-party dependency notices in the approved notice; embedded fonts and icons already have
their notices copied by the helpers. Commit the release source and update the Cargo version for each new app version.

Keep the approved notice inside the repository so the builder can read it. Then, using real values:

```bash
shells/linux/release/container-build.sh deb \
  --revision 1 --copyright-file path/to/approved-notice
shells/linux/release/container-build.sh rpm \
  --revision 1 --copyright-file path/to/approved-notice \
  --license-expression 'PROJECT_APPROVED_SPDX_EXPRESSION'
```

These are command templates: replace the notice path and SPDX expression once approved. Revision `1` is a release
candidate, not an automatic stable publication. Repeat the checks on the final candidate bytes on both architectures.
The helpers refuse production revisions on the wrong OS, with reserved example contacts or without a notice.

## 7. Prepare signed repositories

Choose HTTPS APT and DNF addresses and a project-owned GPG signing key. Keep the private key outside the checkout
and make its ownership, passphrase, backup, expiry and rotation procedure explicit. Use its full fingerprint.
Signing runs on the chosen signing workstation; no private key is needed in the build containers or ARM64 builder.

For APT (requires GPG, dpkg-dev and the qualified packages from both architectures):

```bash
python3 shells/linux/release/apt_repository.py \
  dist-dev/ubuntu24.04-x86_64/optchat_0.1.0-1_amd64.deb \
  dist-dev/ubuntu24.04-aarch64/optchat_0.1.0-1_arm64.deb \
  --key "$OPTCHAT_SIGNING_KEY" --suite stable \
  --output dist-dev/apt-v0.1.0
```

For DNF (requires GPG, RPM 4.19+ including rpmsign/rpmkeys, createrepo_c):

```bash
python3 shells/linux/release/rpm_repository.py \
  dist-dev/fedora43-x86_64/optchat-0.1.0-1.fc43.x86_64.rpm \
  dist-dev/fedora43-aarch64/optchat-0.1.0-1.fc43.aarch64.rpm \
  --key "$OPTCHAT_SIGNING_KEY" --base-url "$OPTCHAT_RPM_URL" \
  --output dist-dev/rpm-v0.1.0
```

The RPM helper signs copies, verifies them using a temporary RPM database, generates per-CPU repositories and signs
`repomd.xml`. The original unsigned build artifacts remain unchanged; distribute the signed RPM copies with their
new checksums. The APT helper signs `InRelease` and `Release.gpg`. Both export the public key and full fingerprint.
Neither uploads files or changes this machine's repository configuration or trusted keys.

Use a private test repo and `--allow-development` for rehearsal packages. Development packages never belong in
the stable customer repository. Test hosted metadata and bad-signature rejection on fresh clients before promotion.

## 8. Publish and give customers native installation instructions

Upload versioned DEB and **signed** RPM files, checksums, changelog and support instructions to a download page or
GitHub Release. A ZIP is unnecessary; customers download the matching package directly. Separately host the
prepared APT/DNF directory contents over HTTPS at the chosen addresses. Deploy payloads and hashed metadata first,
switch signed entry metadata last, and keep older referenced files available during updates.

Publish the signing fingerprint independently from the repository key download. Ubuntu/Debian customer setup uses
a repository-specific `Signed-By` keyring. After setting the real HTTPS `OPTCHAT_APT_URL`, verify the fingerprint
against the independently published value before installing the key:

```bash
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL "$OPTCHAT_APT_URL/optchat-archive-keyring.asc" -o /tmp/optchat-archive-keyring.asc
gpg --show-keys --with-fingerprint /tmp/optchat-archive-keyring.asc
sudo install -m 0644 /tmp/optchat-archive-keyring.asc /etc/apt/keyrings/optchat.asc
printf 'Types: deb\nURIs: %s\nSuites: stable\nComponents: main\nSigned-By: /etc/apt/keyrings/optchat.asc\n' "$OPTCHAT_APT_URL" | sudo tee /etc/apt/sources.list.d/optchat.sources
sudo apt update
sudo apt install optchat
```

The architecture is selected by APT automatically. Fedora uses the generated `optchat.repo` with both package and
metadata signature checks enabled. For Fedora, after verifying the fingerprint:

```bash
curl -fsSL "$OPTCHAT_RPM_URL/optchat-archive-keyring.asc" -o /tmp/OPTCHAT-KEY
gpg --show-keys --with-fingerprint /tmp/OPTCHAT-KEY
sudo install -m 0644 /tmp/OPTCHAT-KEY /etc/pki/rpm-gpg/OPTCHAT-KEY
curl -fsSL "$OPTCHAT_RPM_URL/optchat.repo" -o /tmp/optchat.repo
# Inspect that its HTTPS baseurl is the intended address before installing it.
sudo install -m 0644 /tmp/optchat.repo /etc/yum.repos.d/optchat.repo
sudo dnf install optchat
```

The owner supplies the real repository URLs before these commands are customer-ready. Packages contain only the
client; customers connect it to their engine. Do not distribute fake tokens, development state or a fake engine as
part of a production installation.

## 9. Verify managed updates

Create a higher-version candidate on the same baselines, qualify it, sign it and publish it to a private test channel.
Ubuntu/Debian: `sudo apt update && sudo apt install --only-upgrade optchat`.
Fedora: `sudo dnf upgrade --refresh optchat`.
Confirm the intended version was installed and workspaces/tokens persist. Rehearse recovery to a known-good version
before stable promotion. Customers update through the distro package manager; no app self-updater is required.

## 10. Release only after sign-off

Complete the [publish gate](release/QUALIFICATION.md#publish-gate), record limitations and publish the actual supported OS,
architecture and desktop backends. Packaging helpers and automated smoke tests do not establish production
readiness. AppImage remains available as a separate portable/development channel; the existing Ubuntu 26.04 image
must not be advertised as compatible with the Ubuntu 24.04 minimum.

## Source development and AppImage packaging

The native Rust/GPUI shell runs the shared OptChat UI on Wayland and X11. Linux hooks stay in `src/platform.rs`:
Secret Service token storage (oo7), resident memory from `/proc/self/statm`, and the optional test QR hook.

### Build and run from source

Start from a source checkout containing `shells/linux/run.sh`; use the source commit shown on a successful Linux
workflow run if the default checkout does not include it. Install [rustup](https://rustup.rs/) and the packages
listed below. `rust-toolchain.toml` selects Rust 1.98.1.

```bash
sudo apt install build-essential clang cmake pkg-config libasound2-dev libfontconfig-dev libssl-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libvulkan1 mesa-vulkan-drivers
tools/zed-vendor.sh
cargo run -p optchat-linux
```

`shells/linux/run.sh` prepares the pinned vendor patches and launches the app from any working directory.
The binary is `target/debug/optchat`. A build needs network access the first time and several GB of disk space.
Wayland and X11 are enabled. Vulkan is preferred, with GLES available through GPUI's renderer.

Preferences use `$XDG_CONFIG_HOME/optchat/prefs.json`, or `~/.config/optchat/prefs.json` if XDG_CONFIG_HOME is unset,
empty or relative. `OPTCHAT_PREFS_DIR` overrides that directory for tests.

A running, unlocked Secret Service keyring is required to save engine tokens. Tokens are separate from prefs.
For isolated tests, `OPTCHAT_TOKEN_DIR` uses the existing 0600 file store; this is never enabled by default.
Use Workspaces → Add engine to connect with a scoped test token as described in the root README.

Linux uses Ctrl shortcuts and standard Ctrl+Left/Right, Ctrl+Backspace and Ctrl+Home/End text navigation.
F11 toggles full screen; closing the last window or Ctrl+Q exits the process.
Dictation uses cpal/ALSA; microphone availability depends on your desktop audio configuration.
Camera QR scanning is not implemented. Flatpak is outside the chosen release scope.

### Native packages and production distribution

The agreed minimum is **Ubuntu 24.04**, with qualification on Ubuntu 24.04, Ubuntu 26.04 and Debian 13.
Fedora RPMs are built on Fedora 43 and qualified on Fedora 43 and 44. Both x86_64 and ARM64 are in scope.
Flatpak is outside this release's scope. Native signed APT/DNF repositories provide managed updates, following
[OpenAI's documented Linux distribution model](https://learn.chatgpt.com/docs/linux/linux-app).

Follow the native package procedure above, starting with rootless Podman setup. Then:

```bash
shells/linux/release/container-build.sh deb
shells/linux/release/container-build.sh rpm
```

Run these on each native architecture. They build from source in separate baseline containers and target directories;
the current Ubuntu 26.04 binary is never reused. Outputs go under `dist-dev/ubuntu24.04-<CPU>/` and
`dist-dev/fedora43-<CPU>/`, with checksums and provenance. The first x86_64 baseline DEB/RPM builds require
GLIBC 2.39 and their extracted applications pass 23 smoke checks on each of the host's Wayland and XWayland
backends. The earlier Ubuntu 26.04 development DEB/AppImage requires GLIBC 2.43.
Producing package files does not establish production readiness.

`tests/check-install.sh` checks dependencies/install/removal/reinstall across the agreed OS versions and tests
upgrades when a previous package is supplied. `tests/test-native-package.sh` tests an extracted DEB/RPM with the fake
engine on a real desktop. Clean desktop qualification, secure token persistence, real-engine behavior and final
signed hosted repository checks remain required on both architectures. Use
[the qualification checklist](release/QUALIFICATION.md) to record each OS/CPU result.

`release/package_deb.py` and `release/package_rpm.py` can also run directly on their native baseline systems.
The approved default maintainer is `Fabiha Tasneem <fabiha@reframe.so>`; `--maintainer` or
`OPTCHAT_PACKAGE_MAINTAINER` can override it. Copyright/license work is deferred, with a
[notice template](packaging/COPYRIGHT.template) to complete later. Development packages can be built now;
production revisions require the approved notice and, for RPM, the approved SPDX expression.
`release/apt_repository.py` prepares signed APT metadata;
`release/rpm_repository.py` signs RPM copies and prepares signed DNF metadata. Both need an owner-supplied GPG key
and qualified packages; they do not upload repositories or change this host's package-manager configuration.
Customer installation and update commands are in the distribution guide, using the final HTTPS repository URLs.

### Package an AppImage

Install the packaging prerequisites (in addition to the build prerequisites above):

```bash
sudo apt install curl desktop-file-utils binutils
```

Python 3.11 or newer is required for tool download verification. From the repository root, run:

```bash
shells/linux/release/bundle.sh --check
```

The script also works from another directory when invoked with its absolute path. It:

1. Verifies the pinned linuxdeploy, AppImage output plugin and AppImage runtime in `build/appimage-tools/`.
   Missing, truncated or changed files are downloaded to temporary files and installed only after size and SHA256
   match `release/appimage-tools.json`. The pinned GitHub asset URLs and cache-busting downloads prevent reuse of
   stale partial downloads. If an upstream asset is removed, verify a replacement and update the pins.
2. Builds `optchat-linux` with the stripped `release-lite` Cargo profile and the lockfile.
3. Validates the desktop entry and stages the executable, icon, fonts' license text and shared libraries in a temporary
   AppDir. Fonts and UI icons are already embedded in the executable.
4. Creates an x86_64 AppImage using the verified runtime, with extract-and-run to avoid requiring FUSE during packaging.
5. With `--check`, runs `tests/linux.ts` against the staged AppImage using an isolated fake engine and temporary test
   tokens/preferences. A failed check prevents replacing the previous package in `dist-dev/`.
6. Writes a checksum and build information, then moves all three files into `dist-dev/` and removes the staging directory.

For version 0.1.0, the outputs are:

```text
dist-dev/OptChat-0.1.0-x86_64.AppImage
dist-dev/OptChat-0.1.0-x86_64.AppImage.sha256
dist-dev/OptChat-0.1.0-x86_64.AppImage.build.json
```

The version comes from `[workspace.package]` in the root Cargo.toml. Change that version before the next versioned
release. `CARGO_BUILD_JOBS` controls build parallelism (default 4); `OPTCHAT_PACKAGE_PROFILE=release` selects the
regular release profile. Omitting `--check` builds a package without running the graphical smoke suite. That suite
requires a running desktop and Deno 2.x (`DENO_BIN` may point to its absolute path).

The JSON records the build OS, Rust version, source revision/local changes, maximum GLIBC requirement among the
bundled ELF files, pinned tool hashes and whether the packaged smoke suite passed. An AppImage still uses the host's
graphics drivers, desktop audio services and secure keyring.

#### Compatibility and sharing

The current package is verified on **Ubuntu 26.04 x86_64**. Its bundled binaries reference **GLIBC 2.43**. The package checks pass with normal launch and extract-and-run on Wayland, and with the X11 backend through
XWayland. Other Linux distributions, older Ubuntu versions, aarch64, standalone Xorg sessions and real microphone
recording have not been verified.
For broader support, build the package inside the oldest intended target distribution's VM/container and test it
there and on the other target desktops. A successful build on Ubuntu 26.04 does not establish support for older systems.

On each target desktop, check window rendering and fonts/icons, fake-engine connections, workspace switching,
keyboard shortcuts and reconnect. Separately test adding a scoped test token through Secret Service and reopening
with the same temporary prefs directory; the fake-engine smoke suite uses its isolated file token store. Verify
microphone recording if dictation is part of the release requirements.

To verify an artifact after copying or downloading it:

```bash
cd dist-dev
sha256sum -c OptChat-0.1.0-x86_64.AppImage.sha256
```

Share the AppImage together with its checksum. The build JSON is useful for testers; state the tested build platform
when sharing. Commit `shells/linux/release/`, `shells/linux/packaging/`, the Linux implementation and the updated
README/platform docs. `build/`, `target/` and `dist-dev/` are generated outputs and are already git-ignored. Preference
files and client tokens stay outside the package.

### Verify the package and check usability

Test an existing AppImage without rebuilding it:

```bash
shells/linux/tests/test-package.sh
# Or point the helper at another version:
shells/linux/tests/test-package.sh /absolute/path/to/OptChat.AppImage
```

The helper verifies the matching checksum and runs the 23-check fake-engine suite with normal AppImage launch,
extract-and-run, and the X11 backend when DISPLAY is available. In a Wayland session the X11 test uses XWayland;
it is not a test of a standalone Xorg desktop. The app runs from a temporary directory to catch source-directory
assumptions. The helper needs this checkout and Deno; the customer's AppImage does not need development tools.
All test instances use temporary prefs/tokens and are closed automatically.

For manual checks, open the packaged application with the large fake fixture:

```bash
OPTCHAT_DEMO_BINARY="$PWD/dist-dev/OptChat-0.1.0-x86_64.AppImage" \
OPTCHAT_DEMO_BIG=1 OPTCHAT_DEMO_PORT=18792 \
shells/linux/demo.sh
```

This bypasses Cargo and uses the AppImage. The fixture contains 1,770 entries in the main chat and five child tasks.
Keep the terminal open during the test. Closing the app cleans up that demo's fake engine and temporary data.
Change the port if it is already in use. Use `APPIMAGE_EXTRACT_AND_RUN=1` only to test the FUSE-free fallback;
normal launch should also be checked for customer usability.

Manually inspect fonts/icons, resize the window, toggle full screen, scroll the long chat, switch among the provided
tasks/workspaces, use the real keyboard and clipboard (including multiline and non-ASCII text), open/dismiss dialogs,
and leave it idle for several minutes. Watch for freezes, clipped text, lost drafts and continuing memory growth
when repeating the same actions. Automated probe checks do not establish that scrolling and mouse interaction feel smooth.

Then copy only the AppImage/checksum into a clean desktop VM or another computer on each intended target OS.
Verify launch outside the checkout and without development packages. Use a scoped test-engine token to verify
real connections, secure-store persistence after reopening, attachments and dictation where supported. The fake
engine returns fixed data; it does not verify real AI execution or speech transcription. Record the tested OS,
desktop session, graphics hardware, package checksum and any failures. A local smoke pass alone is not a broad
Linux compatibility claim.

### Local demo and checks

Install Deno 2.x. These commands use only the fake engine on loopback and isolated temporary files:

```bash
shells/linux/demo.sh
cargo build -p optchat-linux
deno run -A tests/linux.ts
cargo test -p optchat-proto -p optchat-theme -p optchat-client -p optchat-linux
```

The demo uses workspaces `alpha` and `beta` on port 18790 (override with `OPTCHAT_DEMO_PORT`). Its fake engine and
temporary preferences/tokens are removed when the app exits. It never reads the real local engine token.
`DENO_BIN` can point to your Deno binary; the demo also finds `~/.deno/bin/deno`.

The smoke suite launches the native binary, verifies rendered frames, the chat snapshot, a shared engine socket,
text entry, Ctrl+A/Ctrl+K/Ctrl+Q, workspace switching, reconnect, and test-token file permissions. It requires a
running graphical desktop and deliberately does not automate other applications.

To run the same checks against a packaged executable, set `OPTCHAT_TEST_BINARY` to its absolute path. For an
AppImage, extract-and-run avoids requiring FUSE:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 \
OPTCHAT_TEST_BINARY="$PWD/dist-dev/OptChat-0.1.0-x86_64.AppImage" \
deno run -A tests/linux.ts
```

If linuxdeploy segfaults even on `--help`, check that its download is complete. Compare its file size and SHA256
with the matching GitHub release asset before executing it. A truncated ELF file can crash before packaging starts.

To verify the desktop keyring with a temporary test entry (created and removed by the test):

```bash
cargo test -p optchat-linux secret_service_round_trip -- --ignored
```

The older macOS UI suites still rely on AppKit mouse injection and macOS bundle launching. `tests/linux.ts` uses
GPUI's existing test command channel for keyboard and application actions.

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
