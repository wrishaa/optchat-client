#!/usr/bin/env bash
# Build native packages in rootless baseline containers, without desktop or keyring access.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
if [ "$#" = 0 ] || [ "${1:-}" = --help ]; then
  echo "Usage: shells/linux/release/container-build.sh deb|rpm [native helper options]"
  echo "Build on Ubuntu 24.04 or Fedora 43 for this machine's native CPU architecture."
  echo "Defaults to a development revision with approved maintainer Fabiha Tasneem <fabiha@reframe.so>."
  echo "Copyright/license work is deferred; --revision 1 needs the approved notice (and RPM SPDX expression)."
  echo "Example: container-build.sh deb --revision 1 --maintainer 'Name <email>' --copyright-file docs/COPYRIGHT"
  exit 0
fi
FORMAT="$1"
shift
case "$FORMAT" in
  deb) BASELINE=ubuntu24.04; BASE_IMAGE=docker.io/library/ubuntu:24.04 ;;
  rpm) BASELINE=fedora43; BASE_IMAGE=registry.fedoraproject.org/fedora:43 ;;
  *) echo "Choose deb or rpm." >&2; exit 2 ;;
esac
ARCH="$(uname -m)"
case "$ARCH" in x86_64|aarch64) ;; *) echo "Only native x86_64 and ARM64 builders are supported." >&2; exit 1 ;; esac
command -v podman >/dev/null || { echo "Install podman uidmap passt fuse-overlayfs first." >&2; exit 1; }
[ "$(id -u)" != 0 ] || { echo "Run this as your regular user, without sudo." >&2; exit 1; }
for arg in "$@"; do
  case "$arg" in
    --check|--binary|--binary=*|--output-dir|--output-dir=*)
      echo "Container builds compile from source and write to a baseline-specific directory. Run desktop checks afterwards." >&2; exit 2 ;;
  esac
done
PACKAGE_ARGS=()
for arg in "$@"; do
  case "$arg" in
    "$ROOT"/*) PACKAGE_ARGS+=("/workspace/${arg#"$ROOT"/}") ;;
    --copyright-file="$ROOT"/*) PACKAGE_ARGS+=("--copyright-file=/workspace/${arg#--copyright-file="$ROOT"/}") ;;
    *) PACKAGE_ARGS+=("$arg") ;;
  esac
done
cd "$ROOT"
# Prepare the pinned submodule on the host; keep-id lets Cargo use the same checkout ownership.
tools/zed-vendor.sh
RUST_VERSION="$(python3 -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml", "rb"))["toolchain"]["channel"])')"
IMAGE="localhost/optchat-build-$BASELINE-$ARCH:$RUST_VERSION"
CACHE="$ROOT/build/native/cargo-$ARCH"
TARGET="$ROOT/build/native/$BASELINE-$ARCH/target"
OUTPUT="$ROOT/dist-dev/$BASELINE-$ARCH"
mkdir -p "$CACHE" "$TARGET" "$OUTPUT"
BUILD_ARGS=(--pull=always)
[ "${OPTCHAT_BUILD_REFRESH:-0}" != 1 ] || BUILD_ARGS+=(--no-cache)
podman build "${BUILD_ARGS[@]}" --build-arg "RUST_VERSION=$RUST_VERSION" \
  --file "shells/linux/release/Containerfile.$BASELINE" --tag "$IMAGE" shells/linux/release
IMAGE_ID="$(podman image inspect --format '{{.Id}}' "$IMAGE")"
podman image inspect "$IMAGE" > "$OUTPUT/builder-image.json"
podman image inspect "$BASE_IMAGE" > "$OUTPUT/base-image.json"
podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" \
  --security-opt=no-new-privileges --cap-drop=all \
  --volume "$ROOT:/workspace:rw" --volume "$CACHE:/cargo:rw" \
  --env CARGO_HOME=/cargo --env "CARGO_TARGET_DIR=/workspace/build/native/$BASELINE-$ARCH/target" \
  --env HOME=/tmp/optchat-home --env "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}" \
  --env "OPTCHAT_BUILDER_IMAGE=$IMAGE" --env "OPTCHAT_BUILDER_IMAGE_ID=$IMAGE_ID" \
  --env OPTCHAT_PACKAGE_MAINTAINER \
  "$IMAGE" bash -c 'mkdir -p "$HOME"; python3 "shells/linux/release/package_$1.py" --output-dir "$2" "${@:3}"' \
  optchat-build "$FORMAT" "/workspace/dist-dev/$BASELINE-$ARCH" "${PACKAGE_ARGS[@]}"
echo "Packages and build evidence: $OUTPUT"
echo "Run graphical and package lifecycle checks in each supported desktop OS before release."
