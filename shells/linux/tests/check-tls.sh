#!/usr/bin/env bash
# Exercise the production transports against temporary TLS fixtures, without a desktop or real credentials.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
ARCH="$(uname -m)"
case "$ARCH" in x86_64|aarch64) ;; *) echo "Use a native x86_64 or ARM64 tester." >&2; exit 1 ;; esac
RUST_VERSION="$(python3 -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml", "rb"))["toolchain"]["channel"])')"
IMAGE="localhost/optchat-build-fedora43-$ARCH:$RUST_VERSION"
podman image exists "$IMAGE" || { echo "Build the baseline RPM first with container-build.sh rpm." >&2; exit 1; }
TARGET="build/native/fedora43-$ARCH/target"
podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" \
  --security-opt=no-new-privileges --cap-drop=all \
  --volume "$ROOT:/workspace:rw" --volume "$ROOT/build/native/cargo-$ARCH:/cargo:rw" \
  --env CARGO_HOME=/cargo --env "CARGO_TARGET_DIR=/workspace/$TARGET" \
  --env HOME=/tmp/optchat-home --env "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}" \
  "$IMAGE" bash -c 'mkdir -p "$HOME"; cargo build --locked --profile release-lite -p optchat-client --example tls-check -j "$CARGO_BUILD_JOBS"'
mkdir -p build/qualification
LOGDIR="$(mktemp -d "$ROOT/build/qualification/$(date -u +%Y%m%dT%H%M%SZ)-tls-$ARCH.XXXXXXXX")"
CLIENT="$ROOT/$TARGET/release-lite/examples/tls-check"
for runtime in "$IMAGE" registry.fedoraproject.org/fedora:44; do
  label=fedora43
  [ "$runtime" != registry.fedoraproject.org/fedora:44 ] || label=fedora44
  python3 tests/linux-tls.py --client "$CLIENT" --podman-image "$runtime" \
    --output "$LOGDIR/$label.json" 2>&1 | tee "$LOGDIR/$label.log"
  podman image inspect "$runtime" > "$LOGDIR/$label-image.json"
done
echo "TLS qualification evidence: $LOGDIR"
