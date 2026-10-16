#!/bin/bash
# tools/zed-vendor.sh: prepare vendor/zed, the GPUI every shell builds against (one pin for all platforms).
# vendor/zed is a git submodule: zed-industries/zed, branch gpui-ios-platform (zed#63068), pinned at ZED_COMMIT.
# Our GPUI patches (patches/zed/*.patch) are applied to its working tree by this script; the root Cargo.toml
# [patch]es every zed crate to vendor/zed/crates/*. Idempotent: re-run after a pull that changes the pin or a patch.
#   ZED_OPTIONAL_PATCHES=1   also apply patches/zed/optional/*.patch (bench instrumentation)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZED="$ROOT/vendor/zed"
ZED_COMMIT=97dc71d8b8f79012c1441aa01121fba95667292a
cd "$ROOT"
if [ ! -e "$ZED/Cargo.toml" ]; then
  # shallow: the pinned commit is the head of the upstream branch, so depth 1 is enough
  git submodule update --init --depth 1 vendor/zed || git submodule update --init vendor/zed
fi
[ "$(git -C "$ZED" rev-parse HEAD)" = "$ZED_COMMIT" ] || { echo "vendor/zed is not at $ZED_COMMIT (git submodule update vendor/zed)" >&2; exit 1; }
# Android's path dependency is resolved by Cargo even for desktop-only builds.
if [ -f "$ROOT/shells/android/prepare-backend.ps1" ]; then
  bash "$ROOT/tools/gpui-mobile-vendor.sh"
fi
PATCHES=("$ROOT"/patches/zed/*.patch)
[ "${ZED_OPTIONAL_PATCHES:-0}" = 1 ] && PATCHES+=("$ROOT"/patches/zed/optional/*.patch)
# already applied (and nothing else changed)? then leave the files alone, so cargo doesn't rebuild GPUI
applied=1; for p in "${PATCHES[@]}"; do git -C "$ZED" apply --reverse --check "$p" 2>/dev/null || applied=0; done
if [ $applied = 1 ] && [ "$(git -C "$ZED" diff --name-only | sort)" = "$(cat "${PATCHES[@]}" | sed -n 's#^+++ b/##p' | sort -u)" ]; then
  exit 0
fi
# start from the pinned tree, then apply our patches
git -C "$ZED" checkout -q -- .
for p in "${PATCHES[@]}"; do git -C "$ZED" apply "$p"; done
echo "vendor/zed at ${ZED_COMMIT:0:7} + ${#PATCHES[@]} patches applied"
