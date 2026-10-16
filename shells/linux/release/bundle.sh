#!/usr/bin/env bash
# Build an AppImage and checksum. --check tests the package before publishing it locally.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  --help|-h)
    echo "Usage: shells/linux/release/bundle.sh [--check]"
    echo "Build an x86_64 Linux AppImage into dist-dev/. --check runs the fake-engine suite."
    exit 0 ;;
  *) echo "Unknown argument: $1" >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo "Expected at most one argument." >&2; exit 2; }
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || {
  echo "This bundler currently supports x86_64 Linux." >&2; exit 1;
}
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
  source "$HOME/.cargo/env"
fi
for tool in cargo python3 curl pkg-config desktop-file-validate readelf sha256sum; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 1; }
done
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' || {
  echo "Packaging requires Python 3.11 or newer." >&2; exit 1;
}
PACKAGE_PROFILE="${OPTCHAT_PACKAGE_PROFILE:-release-lite}"
case "$PACKAGE_PROFILE" in release|release-lite) ;; *) echo "Use release or release-lite." >&2; exit 2 ;; esac
DENO=""
if [ "$CHECK" = 1 ]; then
  DENO="${DENO_BIN:-$(command -v deno || true)}"
  [ -n "$DENO" ] || DENO="$HOME/.deno/bin/deno"
  [ -x "$DENO" ] || { echo "Install Deno 2.x to use --check." >&2; exit 1; }
fi
cd "$ROOT"
tools/zed-vendor.sh
mkdir -p build dist-dev
python3 shells/linux/release/fetch_tools.py
desktop-file-validate shells/linux/packaging/com.reframe.optchat.gpui.desktop
cargo build --locked --profile "$PACKAGE_PROFILE" -p optchat-linux -j "${CARGO_BUILD_JOBS:-4}"
METADATA="$(cargo metadata --locked --no-deps --format-version 1)"
VERSION="$(printf '%s' "$METADATA" | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "optchat-linux"))')"
TARGET_DIR="$(printf '%s' "$METADATA" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
BINARY="$TARGET_DIR/$PACKAGE_PROFILE/optchat"
[ -x "$BINARY" ] || { echo "Expected a native executable at $BINARY; check Cargo target settings." >&2; exit 1; }
STAGE_DIR="$(mktemp -d "$ROOT/build/optchat-package.XXXXXX")"
cleanup() {
  python3 - "$STAGE_DIR" "$ROOT/build" <<'PY'
from pathlib import Path
import shutil
import sys
path, parent = map(lambda p: Path(p).resolve(), sys.argv[1:])
if path.parent != parent or not path.name.startswith("optchat-package."):
    raise SystemExit("Refusing to remove an unexpected staging path")
shutil.rmtree(path)
PY
}
trap cleanup EXIT
APPDIR="$STAGE_DIR/OptChat.AppDir"
APPIMAGE_NAME="OptChat-$VERSION-x86_64.AppImage"
STAGED_APPIMAGE="$STAGE_DIR/$APPIMAGE_NAME"
mkdir -p "$APPDIR/usr/share/licenses/optchat"
cp assets/fonts/*LICENSE* assets/fonts/*OFL* "$APPDIR/usr/share/licenses/optchat/"
cp assets/icons/LICENSE-lucide.txt "$APPDIR/usr/share/licenses/optchat/"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 \
LINUXDEPLOY_OUTPUT_VERSION="$VERSION" \
LDAI_RUNTIME_FILE="$ROOT/build/appimage-tools/runtime-x86_64" \
LDAI_OUTPUT="$STAGED_APPIMAGE" \
build/appimage-tools/linuxdeploy-x86_64.AppImage \
  --appdir "$APPDIR" \
  --executable "$BINARY" \
  --desktop-file shells/linux/packaging/com.reframe.optchat.gpui.desktop \
  --icon-file shells/linux/packaging/assets/com.reframe.optchat.gpui.png \
  --library "$(pkg-config --variable=libdir fontconfig)/libfontconfig.so.1" \
  --library "$(pkg-config --variable=libdir freetype2)/libfreetype.so.6" \
  --output appimage
if [ "$CHECK" = 1 ]; then
  APPIMAGE_EXTRACT_AND_RUN=1 OPTCHAT_TEST_BINARY="$STAGED_APPIMAGE" \
    "$DENO" run -A tests/linux.ts
fi
# Record compatibility facts rather than claiming support for untested distributions.
python3 - "$APPDIR" "$STAGED_APPIMAGE.build.json" "$VERSION" "$PACKAGE_PROFILE" "$CHECK" <<'PY'
import json
from pathlib import Path
import re
import subprocess
import sys

appdir, report, version, profile, checked = sys.argv[1:]
glibc_versions = set()
for path in Path(appdir).rglob("*"):
    if not path.is_file():
        continue
    with path.open("rb") as file:
        if file.read(4) != b"\x7fELF":
            continue
    output = subprocess.check_output(["readelf", "--version-info", str(path)], text=True, stderr=subprocess.DEVNULL)
    glibc_versions.update(re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", output))
os_release = {}
for line in Path("/etc/os-release").read_text().splitlines():
    if "=" in line:
        key, value = line.split("=", 1)
        os_release[key] = value.strip('"')
maximum = max(glibc_versions, key=lambda v: tuple(map(int, v.split(".")))) if glibc_versions else None
info = {
    "application": "OptChat", "version": version, "architecture": "x86_64", "profile": profile,
    "built_on": os_release.get("PRETTY_NAME"), "required_glibc": maximum,
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "source_has_local_changes": bool(subprocess.check_output(["git", "status", "--porcelain"])),
    "packaged_smoke_test": "passed" if checked == "1" else "not_run",
    "tested_on": os_release.get("PRETTY_NAME") if checked == "1" else None,
    "packaging_tools": json.loads(Path("shells/linux/release/appimage-tools.json").read_text()),
}
Path(report).write_text(json.dumps(info, indent=2) + "\n")
print(f"Package GLIBC requirement: {maximum}; built on {info['built_on']}")
PY
(cd "$STAGE_DIR"; sha256sum "$APPIMAGE_NAME" > "$APPIMAGE_NAME.sha256")
for suffix in "" .sha256 .build.json; do
  mv "$STAGED_APPIMAGE$suffix" "$ROOT/dist-dev/$APPIMAGE_NAME$suffix"
done
echo "Built $ROOT/dist-dev/$APPIMAGE_NAME"
echo "Checksum: $ROOT/dist-dev/$APPIMAGE_NAME.sha256"
echo "Build information: $ROOT/dist-dev/$APPIMAGE_NAME.build.json"
