#!/usr/bin/env bash
# Dependency and package lifecycle checks in disposable rootless containers (no desktop session).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
if [ "$#" = 0 ] || [ "${1:-}" = --help ]; then
  echo "Usage: shells/linux/tests/check-install.sh PACKAGE.deb|PACKAGE.rpm [PREVIOUS_PACKAGE]"
  echo "DEB: Ubuntu 24.04, Ubuntu 26.04, Debian 13. RPM: Fedora 43, Fedora 44."
  echo "Checks native architecture, checksums, dependencies, install/remove/reinstall and upgrade if provided."
  echo "Desktop, microphone and Secret Service tests still require real desktop VMs/hardware."
  exit 0
fi
[ "$#" -le 2 ] || { echo "Expected package and optional previous package." >&2; exit 2; }
command -v podman >/dev/null || { echo "Install rootless Podman first." >&2; exit 1; }
[ "$(id -u)" != 0 ] || { echo "Run as your regular user without sudo." >&2; exit 1; }
PACKAGE="$(realpath "$1")"
PREVIOUS=""
[ "$#" = 1 ] || PREVIOUS="$(realpath "$2")"
for path in "$PACKAGE" ${PREVIOUS:+"$PREVIOUS"}; do
  [ -f "$path" ] && [ -f "$path.sha256" ] || { echo "Package/checksum missing: $path" >&2; exit 1; }
  (cd "$(dirname "$path")"; sha256sum -c "$(basename "$path").sha256")
done
case "$PACKAGE" in
  *.deb) FORMAT=deb; IMAGES=(docker.io/library/ubuntu:24.04 docker.io/library/ubuntu:26.04 docker.io/library/debian:13) ;;
  *.rpm) FORMAT=rpm; IMAGES=(registry.fedoraproject.org/fedora:43 registry.fedoraproject.org/fedora:44) ;;
  *) echo "Choose a .deb or .rpm package." >&2; exit 2 ;;
esac
[ -z "$PREVIOUS" ] || [[ "$PREVIOUS" = *.$FORMAT ]] || { echo "Previous package must use the same format." >&2; exit 2; }
ARCH="$(uname -m)"
case "$ARCH" in x86_64|aarch64) ;; *) echo "Use a native x86_64 or ARM64 tester." >&2; exit 1 ;; esac
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$ROOT/build/qualification"
LOGDIR="$(mktemp -d "$ROOT/build/qualification/$STAMP-$FORMAT-$ARCH.XXXXXXXX")"
CHECK_SCRIPT="$LOGDIR/install-check-container.sh"
cp -- "$ROOT/shells/linux/tests/install-check-container.sh" "$CHECK_SCRIPT"
ORIGINAL_PACKAGE="$PACKAGE"
ORIGINAL_PREVIOUS="$PREVIOUS"
snapshot_package() {
  local source="$1" destination="$2"
  mkdir -p "$destination"
  # Save the expected checksum first, then verify the copied bytes before using them.
  cp -- "$source.sha256" "$destination/$(basename "$source").sha256"
  cp --reflink=auto -- "$source" "$destination/$(basename "$source")"
  (cd "$destination"; sha256sum -c "$(basename "$source").sha256")
}
snapshot_package "$PACKAGE" "$LOGDIR/packages/current"
PACKAGE="$LOGDIR/packages/current/$(basename "$PACKAGE")"
if [ -n "$PREVIOUS" ]; then
  snapshot_package "$PREVIOUS" "$LOGDIR/packages/previous"
  PREVIOUS="$LOGDIR/packages/previous/$(basename "$PREVIOUS")"
fi
python3 - "$LOGDIR" "$FORMAT" "$ARCH" "$ORIGINAL_PACKAGE" "$PACKAGE" "$ORIGINAL_PREVIOUS" "$PREVIOUS" "$CHECK_SCRIPT" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
directory, package_format, architecture, original, current, previous_original, previous, check_script = sys.argv[1:]
records = []
for role, source, snapshot in [("current", original, current), ("previous", previous_original, previous)]:
    if not snapshot:
        continue
    path = Path(snapshot)
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    records.append({"role": role, "input_path": source, "snapshot": str(path.relative_to(directory)),
                    "sha256": digest, "size_bytes": path.stat().st_size})
Path(directory, "packages.json").write_text(json.dumps({"format": package_format, "architecture": architecture,
                                                        "check_script_sha256": hashlib.sha256(Path(check_script).read_bytes()).hexdigest(),
                                                        "packages": records}, indent=2) + "\n")
PY
for image in "${IMAGES[@]}"; do
  label="${image##*/}"
  label="${label/:/-}"
  echo "Checking $image ($ARCH)"
  mounts=(--volume "$PACKAGE:/packages/current.$FORMAT:ro")
  [ -z "$PREVIOUS" ] || mounts+=(--volume "$PREVIOUS:/packages/previous.$FORMAT:ro")
  cache="$ROOT/build/qualification-cache/$label-$ARCH"
  if [ "$FORMAT" = deb ]; then
    mkdir -p "$cache/apt-archives" "$cache/apt-lists"
    mounts+=(--volume "$cache/apt-archives:/var/cache/apt/archives:rw" --volume "$cache/apt-lists:/var/lib/apt/lists:rw")
  else
    mkdir -p "$cache/dnf"
    mounts+=(--volume "$cache/dnf:/var/cache/libdnf5:rw")
  fi
  if podman run --rm "${mounts[@]}" --volume "$CHECK_SCRIPT:/check.sh:ro" \
    "$image" bash /check.sh "$FORMAT" "$ARCH" "${PREVIOUS:+yes}" 2>&1 | tee "$LOGDIR/$label.log"; then
    podman image inspect "$image" > "$LOGDIR/$label.image.json"
  else
    echo "FAILED: $image; see $LOGDIR/$label.log" >&2
    exit 1
  fi
done
echo "Container lifecycle checks passed: $LOGDIR"
[ -n "$PREVIOUS" ] || echo "Upgrade NOT TESTED: supply a real previous package as the second argument."
echo "Desktop and real-engine qualification are still required on every OS/architecture."
