#!/usr/bin/env bash
# Cargo resolves every workspace member, including Android's target-specific path dependency.
# Prepare its pinned backend without discarding any local edits.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BACKEND="$ROOT/vendor/gpui-mobile"
PATCH="$ROOT/patches/gpui-mobile/android-app.patch"
PIN=c7cab3a43970bd5f1e05d907695404ed73fcdc95
if [ ! -f "$BACKEND/Cargo.toml" ]; then
  git -c core.autocrlf=false -C "$ROOT" submodule update --init --depth 1 vendor/gpui-mobile
fi
[ "$(git -C "$BACKEND" rev-parse HEAD)" = "$PIN" ] || {
  echo "vendor/gpui-mobile must be at $PIN; preserve local edits before updating it." >&2
  exit 1
}
if git -C "$BACKEND" apply --reverse --check "$PATCH" 2>/dev/null; then
  TEMPORARY="$(mktemp -d -t optchat-mobile-index.XXXXXXXX)"
  trap 'rm -rf -- "$TEMPORARY"' EXIT
  INDEX="$(git -C "$BACKEND" rev-parse --path-format=absolute --git-path index)"
  cp "$INDEX" "$TEMPORARY/index"
  # Include patch-created and unexpected files in the comparison, using only a temporary index.
  GIT_INDEX_FILE="$TEMPORARY/index" git -C "$BACKEND" add --intent-to-add -- .
  GIT_INDEX_FILE="$TEMPORARY/index" git -C "$BACKEND" diff --binary HEAD -- > "$TEMPORARY/actual"
  GIT_INDEX_FILE="$TEMPORARY/index" git -C "$BACKEND" read-tree HEAD
  GIT_INDEX_FILE="$TEMPORARY/index" git -C "$BACKEND" apply --cached "$PATCH"
  GIT_INDEX_FILE="$TEMPORARY/index" git -C "$BACKEND" diff --cached --binary HEAD -- > "$TEMPORARY/expected"
  cmp -s "$TEMPORARY/actual" "$TEMPORARY/expected" || {
    echo "GPUI mobile has additional edits; preserve them before preparing it." >&2
    exit 1
  }
  echo "GPUI mobile pin and Android patch verified."
  exit 0
fi
[ -z "$(git -C "$BACKEND" status --porcelain --untracked-files=normal)" ] || {
  echo "GPUI mobile has local or partially applied edits; preserve them before preparing it." >&2
  exit 1
}
git -C "$BACKEND" apply --check "$PATCH"
git -C "$BACKEND" apply "$PATCH"
echo "Prepared GPUI mobile for the shared workspace."
