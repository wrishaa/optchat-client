# OptChat for Windows

Native desktop client for connecting to OptChat engines and workspaces.

**Release status:** These are development test builds. The x64 installer has a development signature;
the ARM64 CI installer is unsigned. Public release approval is pending trusted publisher signing and
platform qualification. The project license and dependency redistribution review are deferred.

## Install

1. Choose the test installer for your PC: [Intel/AMD x64](../../releases/windows/x64/OptChat-0.1.0.4-windows-x64-Setup.exe)
   or [Windows 11 ARM64](../../releases/windows/arm64/OptChat-0.1.0.1-windows-arm64-Setup.exe).
2. Open the installer and select **Install**.
3. Launch **OptChat** from the Start menu.

OptChat installs for the current Windows account in `%LOCALAPPDATA%\Programs\OptChat`.
Administrator access is not required. A desktop shortcut is optional.

Windows 10 22H2 or later and a DirectX-compatible graphics driver are required.
ARM64 packages use `arm64` in the filename and require Windows 11.
Check **Settings > System > About > System type** to identify your PC's architecture.
The architecture folders contain SHA256 checksums and build reports. The ARM64 folder also contains an
optional portable ZIP. The x64 and ARM64 snapshots have separate development revision histories;
install updates for your own architecture. The [qualification record](release/QUALIFICATION.md) identifies
the source and checks for each snapshot.

## Connect to an engine

1. Obtain a client token from your OptChat engine administrator.
2. Open **Workspaces > Add engine** in OptChat.
3. Paste the token and select **Add**.

The workspaces authorized by the token connect automatically. OptChat requires a separately running engine.
Tokens are stored in Windows Credential Manager; preferences are stored in `%APPDATA%\OptChat`.

## Update

Open **Settings > App > Download updates** or visit [Windows releases](https://github.com/fabihatasneem/optchat-client/releases).
Download the newer installer for your PC, close OptChat, and run it. Existing preferences and credentials
are retained. Updates are installed manually; an older installer cannot replace a newer installation.
Published production downloads will appear on the releases page after approval.

## Uninstall

Open **Settings > Apps > Installed apps > OptChat > Uninstall**.
Preferences and credentials are retained. To remove saved tokens, remove them in OptChat before uninstalling.

## Portable edition

The portable ZIP and managed MSIX are optional packages, separate from the standard installer.
When a ZIP is provided, extract `OptChat-<version>-windows-x64-portable.zip` and open `OptChat\optchat.exe`.
Keep the extracted folder together. To update, close the app and extract the newer package into a new folder.
To remove the portable edition, delete its application folder.

Use one installation method at a time. To switch from an MSIX installation to Setup.exe, close OptChat and
uninstall the existing OptChat entry in Windows Settings first. This installer does not migrate MSIX registrations.

## Build and release

See [BUILDING.md](BUILDING.md) for compilation, packaging and signing, and
[release/QUALIFICATION.md](release/QUALIFICATION.md) for release validation.
