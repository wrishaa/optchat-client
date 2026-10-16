#!/usr/bin/env bash
# Verify an existing AppImage using isolated fake engines and an empty app working directory.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
if [ "${1:-}" = --help ]; then
  echo "Usage: shells/linux/tests/test-package.sh [/absolute/path/to/OptChat.AppImage]"
  echo "Check checksum, normal launch, extract-and-run, and the available X11 backend."
  exit 0
fi
[ "$#" -le 1 ] || { echo "Expected at most one AppImage path." >&2; exit 2; }
DENO="${DENO_BIN:-$(command -v deno || true)}"
[ -n "$DENO" ] || DENO="$HOME/.deno/bin/deno"
[ -x "$DENO" ] || { echo "Install Deno 2.x to run package tests." >&2; exit 1; }
VERSION="$(python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["workspace"]["package"]["version"])' "$ROOT/Cargo.toml")"
PACKAGE="$(realpath "${1:-$ROOT/dist-dev/OptChat-$VERSION-x86_64.AppImage}")"
[ -x "$PACKAGE" ] || { echo "Missing executable AppImage: $PACKAGE" >&2; exit 1; }
[ -f "$PACKAGE.sha256" ] || { echo "Missing checksum: $PACKAGE.sha256" >&2; exit 1; }
(cd "$(dirname "$PACKAGE")"; sha256sum -c "$(basename "$PACKAGE").sha256")
cd "$ROOT"
echo "Checking normal AppImage launch on the current desktop"
env -u APPIMAGE_EXTRACT_AND_RUN -u OPTCHAT_HEADLESS \
  OPTCHAT_TEST_BINARY="$PACKAGE" "$DENO" run -A tests/linux.ts
echo "Checking AppImage extract-and-run launch"
env -u OPTCHAT_HEADLESS APPIMAGE_EXTRACT_AND_RUN=1 \
  OPTCHAT_TEST_BINARY="$PACKAGE" "$DENO" run -A tests/linux.ts
if [ -n "${DISPLAY:-}" ]; then
  echo "Checking the X11 backend (XWayland when running in a Wayland session)"
  env -u WAYLAND_DISPLAY -u APPIMAGE_EXTRACT_AND_RUN -u OPTCHAT_HEADLESS \
    OPTCHAT_TEST_BINARY="$PACKAGE" "$DENO" run -A tests/linux.ts
else
  echo "X11 check skipped: DISPLAY is not set"
fi
echo "Package checks passed. Manual usability and clean-machine testing are still required."
