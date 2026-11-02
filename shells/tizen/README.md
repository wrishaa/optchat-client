# shells/tizen: optchat as a native app on samsung tizen tvs

The shared optchat UI (`crates/ui`, the same `Workspace` the mac and ios shells use) running natively on a samsung tv,
through GPUI's wayland backend and wgpu's gles backend. Proved on a **QA55Q60C** (tizen 9.0, 2023 Q60C, Mali-G31,
32-bit armv7 userland) with the samsung smart remote (mic button + d-pad).

## shape

```
host/      tiny c# (.net, Tizen.NET.Sdk, tizen80) app: Main() calls the rust side (oc_main) and owns the remote's mic
           (bluetooth hid voice: bt_hid_rc_start_sending_voice, 16 kHz s16le mono, the same path webapis.microphone uses)
src/       cdylib libocgpui.so (armv7-unknown-linux-gnueabi): lib.rs (shell + exports), platform.rs (token store in a
           0600 file in the app data dir, prefs dir, mic hand-off), dev.rs (dev-only test channel, feature dev-capture)
toolchain/ tcc (clang wrapper: tizen studio llvm-10 + rootstrap 9.0 + arm gcc 14.2 crt), xkb-stub.c (link stub; the
           real libxkbcommon.so.0 is on the tv)
vendor/wgpu-hal  wgpu-hal with one change: legacy eglGetDisplay(wl_display) when EGL_EXT_platform_wayland is missing
../../patches/zed/tizen-tv.patch  gpui on the tv: zxdg_shell_v6 fallback (no xdg_wm_base), wl_seat v4,
           uniform-buffer instance path (Mali-G31 has no vertex storage buffers), tizen_keyrouter grabs for the remote's
           keys (mic, color keys, back), optional frame capture (dev)
```

Why .net and not a plain native tpk: on a retail tv a plain native c tpk installs but never runs anything we could see;
a .net tpk runs and can `DllImport` a bundled `.so` (the flutter-tizen route).

## what you need

1. a mac (or linux) with rust (`rustup target add armv7-unknown-linux-gnueabi`), the .net sdk (`~/.dotnet/dotnet` or
   `DOTNET=...`), and **tizen studio** (cli is enough) at `~/tizen-studio` (or `TIZEN_STUDIO=...`) with:
   tv extension 9.0 rootstrap (`platforms/tizen-9.0/tizen/rootstraps/tizen-9.0-device.core`), the
   `arm-linux-gnueabi-gcc-14.2` toolchain and `llvm-10`, and the samsung certificate extension.
2. the tv in developer mode with your computer's ip as the host (see below), reachable over sdb.
3. **your own samsung certificate profile** (certificate manager, "samsung" type, tv, signed in with your samsung
   account, the tv's duid added). certs and passwords never go in this repo (`.gitignore` has `*.p12`, `*.pem`, ...).
4. an optchat engine + a token for it (the tv has no keyboard to paste a token: see "first run" below).

## build, sign, install, run

```sh
tools/zed-vendor.sh                       # vendor/zed at the pin + patches/zed/*.patch (incl. tizen-tv.patch)
TIZEN_PROFILE=<your cert profile> \
TIZEN_SERIAL=<sdb serial, eg. 192.0.2.50:26101> \
  sh shells/tizen/build.sh                # cargo cross build + strip + dotnet tpk + re-sign + tizen install + run
```

Flags: `--no-install` (package only), `--seed FILE` (first run, below), `--dev-capture --dev-host <your mac ip>` (dev
builds only: screenshots, test channel, fake mic, logs to dev servers on the mac, never ship), `--theme-preview`
(temporary: cycle all bundled themes). The app id is `so.reframe.octv`, label "OptChat (native)".

macos note: if `sdb connect <tv-ip>` hangs or is refused, macos "local network" permission is blocking the cli. a tiny
tcp relay you start from a terminal that has the permission (127.0.0.1:26111 -> tv:26101), then
`sdb connect 127.0.0.1:26111` works; the serial is then `emulator-26111`.

## first run (engine + token)

`--seed FILE` puts `FILE` (json `{"engines":[{"endpoint","name","ws","token"}],"active":0}`) into the package as
`res/dev-seed.json` for one install: the app copies the engine + token into its own data dir (token file 0600) and
the seed is removed from the build tree right after packaging. Keep the seed file 0600, outside the repo; use a token
made for the tv only, so you can revoke it alone. Later installs keep the token (it lives in the app's data dir).

## remote keys

d-pad moves focus, OK selects / sends, back goes back, **hold the mic button** to dictate (release ends it; the tail the
remote buffered is kept), red deletes a word, green sends. The mic key is grabbed through `tizen_tv_keyrouter`, so bixby
doesn't open while the app is in front.

## known gaps (2026-11-02)

- perf: ~3 fps at 1080p on the Mali-G31 / a72-32bit (frame ~300 ms: ~125 ms gpui layout+paint, ~165 ms gles draw); it
  also redraws while idle while a spinner is visible.
- sidebar on the tv shows only running/paused tasks, their ancestors and the open task (`crates/ui/src/tasks_view.rs`).
- no on-screen keyboard: text comes from dictation only.
