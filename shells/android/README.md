# OptChat for Android

OptChat's Android development build runs the shared Rust client and GPUI phone interface.
It connects to your OptChat engine using a client token. The hello-window experiment has been replaced.

## Install

1. Download [OptChat 0.1.0 development APK for ARM64](../../releases/android/OptChat-0.1.0-dev-arm64.apk).
2. Open the APK and allow installation from the download app when Android requests it.
3. Open **OptChat**.

The package is development-signed. The [checksum](../../releases/android/OptChat-0.1.0-dev-arm64.apk.sha256)
and [build information](../../releases/android/build-info.json) accompany it. It is not published on Google Play.
The tested device is a OnePlus 9 5G running Android 14; broader compatibility is not yet qualified.

## Connect your engine

1. Get a client token from **Client tokens** on your engine administrator's `/engine` page.
2. On the app's **Workspaces** screen, tap **Add engine** and paste the `oc1.…` token.
3. Optionally enter a name, then tap **Add**. Workspaces permitted by the token connect automatically.

Android encrypts saved tokens with a key held in Android Keystore. Keep tokens private.
To reach Workspaces again, tap the workspace avatar at the top left, or open **More → Workspaces**.
The microphone permission is requested when you first use dictation; after granting it, retry dictation.
QR scanning is not available in this build.

See the [device qualification](QUALIFICATION.md) for checked behavior and remaining limitations.
Developers can use the [build and test guide](BUILDING.md) and [backend research](RESEARCH.md).
