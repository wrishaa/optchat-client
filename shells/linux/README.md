# Install OptChat on Linux

OptChat is a desktop app for connecting to an OptChat engine. You will need a client token from your engine
administrator to load your workspaces. Installing this app does not install the engine.

**Current status: development preview.** Prebuilt DEB and RPM packages are available as direct
[GitHub Release downloads](https://github.com/fabihatasneem/optchat-client/releases/tag/linux-v0.1.0-preview.7).
A stable Linux release and public APT/DNF update repositories have not been published yet. Package
installation has been tested across the systems below; full desktop and real-engine qualification is still in progress.
For preview testing, ask for a test workspace.

You need a graphical Linux desktop, internet access for dependencies and engine connections, permission to install
software with `sudo`, and an unlocked desktop password wallet/keyring for saving your engine token.

## 1. Choose the package for your computer

Check your distribution and version in your system's **About** settings. You can also run:

```bash
cat /etc/os-release
uname -m
```

The first command shows the system name/version, including `PRETTY_NAME`. The second shows the processor type.
Choose one download:

| Linux distribution | Processor (`uname -m`) | Direct download |
|---|---|---|
| Ubuntu 24.04/26.04, Debian 13 | `x86_64` | [OptChat .deb — Intel / AMD 64-bit](https://github.com/fabihatasneem/optchat-client/releases/download/linux-v0.1.0-preview.7/optchat_0.1.0-0local.7_amd64.deb) |
| Ubuntu 24.04/26.04, Debian 13 | `aarch64` / `arm64` | [OptChat .deb — ARM64](https://github.com/fabihatasneem/optchat-client/releases/download/linux-v0.1.0-preview.7/optchat_0.1.0-0local.7_arm64.deb) |
| Fedora 43/44 | `x86_64` | [OptChat .rpm — Intel / AMD 64-bit](https://github.com/fabihatasneem/optchat-client/releases/download/linux-v0.1.0-preview.7/optchat-0.1.0-0local.7.fc43.x86_64.rpm) |
| Fedora 43/44 | `aarch64` / `arm64` | [OptChat .rpm — ARM64](https://github.com/fabihatasneem/optchat-client/releases/download/linux-v0.1.0-preview.7/optchat-0.1.0-0local.7.fc43.aarch64.rpm) |

`amd64` means the standard 64-bit Intel/AMD package; it works on both brands. `arm64` and `aarch64` name the
64-bit ARM architecture. Other distributions/versions and 32-bit machines are outside this package test matrix.

## 2. Download the installer

If you already have a source checkout, the same `0local.7` preview installers are in
`releases/linux/x86_64/` for Intel/AMD or `releases/linux/aarch64/` for ARM64, from the repository root.
Use the `.deb` on Ubuntu/Debian or `.rpm` on Fedora; each has an adjacent checksum.
You can continue at step 3 using that folder.

1. Select the matching `.deb` or `.rpm` link above and save the file.
2. Continue with installation below. Keep only one OptChat installer in that folder.

For optional download verification, open the
[release page](https://github.com/fabihatasneem/optchat-client/releases/tag/linux-v0.1.0-preview.7)
and save the matching `.sha256` file from **Assets** to the same folder. Only the `.deb` or `.rpm` is needed
for installation.

The package downloads directly, without a GitHub sign-in or ZIP extraction. Release assets remain available
independently of the temporary GitHub Actions artifacts. The release also includes build provenance for maintainers.

## 3. Install

Open a terminal **in the folder containing the installer**. Many file managers offer
**Open in Terminal** when you right-click an empty area. That folder should contain only one OptChat installer.

### Ubuntu or Debian

If you downloaded the matching checksum, verify it first:

```bash
sha256sum -c ./optchat_*.deb.sha256
```

The check must end in **OK**. To install the package:

```bash
sudo apt update
sudo apt install ./optchat_*.deb
```

### Fedora

If you downloaded the matching checksum, verify it first:

```bash
sha256sum -c ./optchat-*.rpm.sha256
```

The check must end in **OK**. To install the package:

```bash
sudo dnf install ./optchat-*.rpm
```

The package manager downloads required system libraries and installs the app. Review the proposed changes and
confirm when prompted. When `sudo` asks for your password, it is your Linux account password; the terminal may
show no characters while you type it.

The package name is **`optchat`**, although the source repository is named `optchat-client`. The `./` in the install
command tells the package manager to use your downloaded file. Installing by name alone needs a configured OptChat
repository, which is not available yet. These preview packages are unsigned; if your system requires a signed RPM,
request an approved signed package from the maintainer rather than disabling signature checks.

## 4. Open OptChat and add your engine

1. Open your desktop's application menu, search for **OptChat**, and launch it. You can also run `optchat` in a
   terminal as your regular user.
2. Ask your engine administrator for a client token for your permitted workspaces. It starts with `oc1.`.
   If you manage the engine, open its `/engine` admin page and create a token under **Client tokens**.
3. In OptChat, open **Workspaces → Add engine**, paste the token and add the engine. Its permitted workspaces
   connect automatically.
4. Quit and reopen OptChat to check that your desktop keyring has saved the token. If the desktop asks you to
   unlock its password wallet, unlock it to allow secure token storage.

Do not launch the app with `sudo`. An empty workspace list before adding an engine is expected. Keep client tokens
private; do not include them in screenshots, support messages or logs.

Useful shortcuts: **Ctrl+K** opens the command palette, **F11** toggles full screen, and **Ctrl+Q** quits.

## Updating and removing the app

For now, check [GitHub Releases](https://github.com/fabihatasneem/optchat-client/releases) for a newer Linux preview
for the same system/processor, verify its checksum and repeat the matching
installation command above. Quit OptChat before updating. Downloads do not update automatically, and ordinary
system updates will not fetch OptChat updates until a signed OptChat package repository is published and configured.

To uninstall on Ubuntu/Debian:

```bash
sudo apt remove optchat
```

On Fedora:

```bash
sudo dnf remove optchat
```

Removing the package keeps your preferences and engine tokens. Preferences normally live in
`~/.config/optchat`; tokens are stored separately in your desktop keyring.

## If something does not work

- **Download link does not open:** open the release page and expand **Assets**, then select the matching package.
- **Checksum says FAILED or a file is missing:** download the package and its matching checksum again.
  Run the command from their shared folder; do not continue installing a file that fails the check.
- **Wrong architecture / cannot execute the file:** repeat `uname -m` and choose the matching download from step 1.
- **Unable to locate package:** use the downloaded file with `./` as shown above. `optchat-client` is not the
  installed package name.
- **Missing libraries or a GLIBC error:** check your OS version and package choice. Use the supported DEB/RPM for
  your system. Earlier experimental AppImages require newer Ubuntu 26.04 libraries and have different compatibility.
- **Token does not stay saved:** make sure your desktop password wallet/keyring is running and unlocked. GNOME
  Keyring is one supported Secret Service provider. Do not use a plain-text token file as a workaround.
- **The window does not open:** run `optchat` in a terminal on your graphical desktop and copy the error text for
  the maintainer. The app needs working graphics drivers; installing a package in a headless server or SSH session
  does not create a desktop session.
- **No workspaces appear after adding the engine:** confirm with the administrator that the token is valid, grants
  workspace access and points to an engine reachable from your computer.

For help, contact the person who supplied your installer or
[Fabiha Tasneem](mailto:fabiha@reframe.so). Include your Linux version, `uname -m` result, package filename and
the error text. Leave out client tokens and other credentials.

## Build and release

See [BUILDING.md](BUILDING.md) for source builds and packaging, and
[release/QUALIFICATION.md](release/QUALIFICATION.md) for release validation.
