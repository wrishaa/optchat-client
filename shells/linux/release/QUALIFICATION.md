# Linux release qualification

Release version: ______  Source commit: ______  Package revision: ______

Approved maintainer contact: Fabiha Tasneem <fabiha@reframe.so>

Approved license/notice: deferred; complete [the notice template](../packaging/COPYRIGHT.template) later.

APT URL: ______  DNF URL: ______  Published signing fingerprint: ______

## Agreed scope

- DEB built natively on Ubuntu 24.04 for amd64 and arm64.
- Test that same DEB on Ubuntu 24.04, Ubuntu 26.04 and Debian 13, on both architectures.
- RPM built natively on Fedora 43 for x86_64 and aarch64.
- Test that same RPM on Fedora 43 and Fedora 44, on both architectures.
- Native signed APT/DNF updates. Flatpak is outside this release's scope.

Copy the checklist below into a release record once per OS/architecture (ten records).
Containers cover package lifecycle; desktop VMs/hardware cover graphics, audio, keyrings and real-engine behavior.
Record an explicit exception for an untested display backend; do not claim it as supported without evidence.

## Per-system record

OS/version: ______  CPU: ______  Desktop/version: ______

Display backend (Wayland, standalone Xorg, or XWayland): ______

GPU/driver: ______  Audio/portal/keyring implementation: ______

Package filename: ______  SHA256: ______  Tester/date: ______

- [ ] Build report confirms the required baseline and native architecture; source is committed.
- [ ] Adjacent checksum passes; maintainer/license/embedded asset and dependency notices are approved.
- [ ] Lintian/Rpmlint findings are reviewed; no unresolved installation/security errors.
- [ ] Installation uses `apt install ./package.deb` or `dnf install ./package.rpm` on a clean system with no build tools.
- [ ] All dependencies resolve; package files belong to root and have suitable modes.
- [ ] Application appears with the correct name/icon and launches from the application menu.
- [ ] `optchat` launches from a terminal in an empty working directory.
- [ ] Extracted package fake-engine smoke checks pass (23 checks per tested backend).
- [ ] Typing, selection, navigation shortcuts, copy/paste and workspace/task switching work manually.
- [ ] File attachments/open/save flows and desktop portals work.
- [ ] Scoped real-engine connection, streaming, cancellation, errors and reconnect work.
- [ ] HTTPS/WSS certificate and hostname verification are tested; Fedora crypto-policy behavior is qualified or documented as a limitation.
- [ ] Add an engine through the normal UI; its token persists through quit/relaunch and desktop logout/reboot.
- [ ] Normal launch uses an unlocked Secret Service; unavailable/locked keyring failures are understandable.
- [ ] Dictation works with a real microphone; denial and unavailable-device behavior are understandable.
- [ ] Large histories remain responsive; resize, full screen, close and Ctrl+Q work without an orphan process.
- [ ] Offline startup/reconnect and suspended/resumed networking behave correctly.
- [ ] Upgrade from the actual previous version succeeds and preserves preferences and engine tokens.
- [ ] Removal (DEB also purge), then reinstall, removes system files and preserves user data.
- [ ] Signed hosted APT/DNF repository is accepted by a fresh client with the published key.
- [ ] Package-manager update installs the intended newer version; user data remains usable.
- [ ] Bad/missing signatures and tampered packages are rejected by the package manager.
- [ ] Logs contain no client tokens; credentials/test data are absent from published artifacts.

Notes, limitations, failure links and log paths: ______

## First release

There is no genuine previous production version yet. Record that explicitly. Test the upgrade mechanism with two
development revisions built from the baseline: `--revision 0local`, then `--revision 0local.1`.
Pass the newer package first and older package second to `shells/linux/tests/check-install.sh`. A private test repository can use
`--allow-development` to rehearse managed updates. Production repositories reject these development revisions.
This exercises the package manager without inventing evidence of upgrading an earlier customer release.

## Publish gate

- [ ] All ten OS/architecture records are complete and reviewed, with documented backend support/limitations.
- [ ] Versioned DEB/RPM, checksums, source revision/build evidence, changelog and install/update instructions are ready.
- [ ] Signed APT indices and RPM/repomd signatures verify; release signing key ownership/backup/expiry are settled.
- [ ] HTTPS repositories serve correct paths/content and retain older payloads and hashed indices during deployments.
- [ ] Stable-channel update and rollback/recovery procedure has been rehearsed.

A generated package, headless container pass, or fake-engine pass alone does not complete this checklist.
