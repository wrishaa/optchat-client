#!/usr/bin/env bash
# Run the existing fake-engine suite against an extracted DEB/RPM on a real desktop.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
if [ "$#" = 0 ] || [ "${1:-}" = --help ]; then
  echo "Usage: shells/linux/tests/test-native-package.sh /path/to/optchat.deb|optchat.rpm"
  echo "Verify checksum, extract to a temporary directory, and test available desktop backends."
  echo "This does not install the package or test the production Secret Service/real engine."
  exit 0
fi
[ "$#" = 1 ] || { echo "Expected one native package path." >&2; exit 2; }
PACKAGE="$(realpath "$1")"
[ -f "$PACKAGE.sha256" ] || { echo "Adjacent checksum missing." >&2; exit 1; }
(cd "$(dirname "$PACKAGE")"; sha256sum -c "$(basename "$PACKAGE").sha256")
DENO="${DENO_BIN:-$(command -v deno || true)}"
[ -n "$DENO" ] || DENO="$HOME/.deno/bin/deno"
[ -x "$DENO" ] || { echo "Install Deno 2.x for desktop smoke tests." >&2; exit 1; }
TEMPORARY="$(mktemp -d -t optchat-native-check.XXXXXXXX)"
cleanup() {
  python3 - "$TEMPORARY" <<'PY'
from pathlib import Path
import shutil
import tempfile
import sys
path = Path(sys.argv[1]).resolve()
if path.parent != Path(tempfile.gettempdir()).resolve() or not path.name.startswith('optchat-native-check.'):
    raise SystemExit('Unexpected test directory')
shutil.rmtree(path)
PY
}
trap cleanup EXIT
case "$PACKAGE" in
  *.deb) dpkg-deb --extract "$PACKAGE" "$TEMPORARY" ;;
  *.rpm)
    if command -v rpm2cpio >/dev/null && command -v cpio >/dev/null; then
      (cd "$TEMPORARY"; rpm2cpio "$PACKAGE" | cpio -id --quiet --no-absolute-filenames --no-preserve-owner)
    else
      RUST_VERSION="$(python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["toolchain"]["channel"])' "$ROOT/rust-toolchain.toml")"
      IMAGE="localhost/optchat-build-fedora43-$(uname -m):$RUST_VERSION"
      command -v podman >/dev/null && podman image exists "$IMAGE" || {
        echo "Install rpm2cpio and cpio, or prepare the native Fedora builder with container-build.sh rpm." >&2
        exit 1
      }
      podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" \
        --security-opt=no-new-privileges --cap-drop=all \
        --volume "$PACKAGE:/package.rpm:ro" --volume "$TEMPORARY:/extracted:rw" \
        "$IMAGE" bash -o pipefail -c \
        'cd /extracted; rpm2cpio /package.rpm | cpio -id --quiet --no-absolute-filenames --no-preserve-owner'
    fi
    ;;
  *) echo "Choose a .deb or .rpm package." >&2; exit 2 ;;
esac
BINARY="$TEMPORARY/usr/bin/optchat"
[ -x "$BINARY" ] || { echo "Extracted launcher is missing." >&2; exit 1; }
env -u OPTCHAT_HEADLESS OPTCHAT_TEST_BINARY="$BINARY" "$DENO" run -A "$ROOT/tests/linux.ts"
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ -n "${DISPLAY:-}" ]; then
  env -u OPTCHAT_HEADLESS -u WAYLAND_DISPLAY OPTCHAT_TEST_BINARY="$BINARY" "$DENO" run -A "$ROOT/tests/linux.ts"
fi
echo "Extracted package desktop smoke checks passed on this machine."
