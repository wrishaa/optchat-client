# Linux installers

For Ubuntu or Debian on an Intel/AMD 64-bit computer, use the **[.deb installer](x86_64/optchat_0.1.0-0local.7_amd64.deb)**.
It is in `releases/linux/x86_64/` from the repository root. ARM64 packages are in `releases/linux/aarch64/`.

- Ubuntu 24.04/26.04 or Debian 13: [Intel/AMD DEB](x86_64/optchat_0.1.0-0local.7_amd64.deb) or [ARM64 DEB](aarch64/optchat_0.1.0-0local.7_arm64.deb).
- Fedora 43/44: [Intel/AMD RPM](x86_64/optchat-0.1.0-0local.7.fc43.x86_64.rpm) or [ARM64 RPM](aarch64/optchat-0.1.0-0local.7.fc43.aarch64.rpm).

Check `uname -m`: `x86_64` means Intel/AMD; `aarch64` means ARM64. Open the matching architecture folder
in your file manager, right-click an empty area, and choose **Open in Terminal**.
Use the commands for your system:

## Ubuntu or Debian

```bash
sha256sum -c ./optchat_*.deb.sha256
sudo apt update
sudo apt install ./optchat_*.deb
```

## Fedora

```bash
sha256sum -c ./optchat-*.rpm.sha256
sudo dnf install ./optchat-*.rpm
```

The checksum must say **OK** before installing. Launch **OptChat** from your application menu, then choose
**Workspaces → Add engine** and paste the client token supplied by your engine administrator.
See the [Linux installation guide](../../shells/linux/README.md) for keyring setup, updates and troubleshooting.

## Build information

These are unsigned `0local.7` development packages from the published `linux-v0.1.0-preview.7` release,
built from committed source `78d8c1d`. They have not been rebuilt from the merged source.
Each checksum was verified when importing the packages. The original reports are preserved beside each
installer, with the complete [release manifest](linux-release.json) recording their successful native CI run.
Build reports record state at package creation; later lifecycle results are in the linked CI run.

For rebuilding and refreshing these files, see
the [maintainer guide](../../shells/linux/BUILDING.md). Keep each installer, checksum and build report together
and verify the checksum after copying a new build into this directory.
