#!/usr/bin/env bash
# An isolated local demo; every engine, preference and token here is test data.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DENO="${DENO_BIN:-$(command -v deno || true)}"
[ -n "$DENO" ] || DENO="$HOME/.deno/bin/deno"
[ -x "$DENO" ] || { echo "Install Deno 2.x to run the fake engine." >&2; exit 1; }
cd "$ROOT"
DEMO_BINARY="${OPTCHAT_DEMO_BINARY:-}"
if [ -z "$DEMO_BINARY" ]; then
  if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
  fi
  tools/zed-vendor.sh
  cargo build -p optchat-linux -j "${CARGO_BUILD_JOBS:-4}"
  DEMO_BINARY="${CARGO_TARGET_DIR:-target}/debug/optchat"
fi
[ -x "$DEMO_BINARY" ] || { echo "No executable at $DEMO_BINARY" >&2; exit 1; }
DEMO_DIR="$(mktemp -d "${TMPDIR:-/tmp}/optchat-linux-demo.XXXXXX")"
PORT="${OPTCHAT_DEMO_PORT:-18790}"
ENGINE_PID=""
cleanup() {
  if [ -n "$ENGINE_PID" ]; then
    kill "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  rm -rf "$DEMO_DIR"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
printf '%s\n' '{"alpha":"linux-demo-token","beta":"linux-demo-token"}' > "$DEMO_DIR/test-tokens.json"
chmod 600 "$DEMO_DIR/test-tokens.json"
FAKE_ARGS=("$PORT" alpha beta)
[ "${OPTCHAT_DEMO_BIG:-0}" != 1 ] || FAKE_ARGS+=(--big)
"$DENO" run --allow-net=127.0.0.1 tests/lib/fake-engine.ts "${FAKE_ARGS[@]}" > "$DEMO_DIR/engine.log" 2>&1 &
ENGINE_PID=$!
READY=0
for attempt in {1..50}; do
  if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
    cat "$DEMO_DIR/engine.log" >&2
    exit 1
  fi
  if grep -q "^fake engine on $PORT$" "$DEMO_DIR/engine.log" && curl -fsS "http://127.0.0.1:$PORT/engine/workspaces" >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 0.1
done
[ "$READY" = 1 ] || { cat "$DEMO_DIR/engine.log" >&2; echo "Fake engine did not start." >&2; exit 1; }
OPTCHAT_PREFS_DIR="$DEMO_DIR" \
OPTCHAT_TOKEN_DIR="$DEMO_DIR" \
OPTCHAT_TEST_TOKENS="$DEMO_DIR/test-tokens.json" \
OPTCHAT_LOCAL_TOKEN_FILE="$DEMO_DIR/no-local-token" \
OPTCHAT_TEST_NO_GLOBAL=1 \
OPTCHAT_ENGINE="http://127.0.0.1:$PORT" \
OPTCHAT_ENGINE_NAME="Local demo" \
OPTCHAT_CONNECT=alpha,beta \
"$DEMO_BINARY"
