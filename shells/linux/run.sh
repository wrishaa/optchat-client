#!/usr/bin/env bash
# Build and launch the native Linux app from any directory.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
  source "$HOME/.cargo/env"
fi
cd "$ROOT"
tools/zed-vendor.sh
exec cargo run -p optchat-linux -j "${CARGO_BUILD_JOBS:-4}"
