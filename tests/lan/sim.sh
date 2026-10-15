#!/bin/bash
# Lan-first in the iOS simulator: installs the side-by-side test app (com.reframe.optchat.lantest) on its own
# simulator, seeds an engine token, and collects the app's lan log lines (Documents/diag.log).
#   tests/lan/sim.sh fake [bench-n]   a fake engine (127.0.0.1:18791) as the "tunnel" + a lan front copy on :18961
#   tests/lan/sim.sh real [bench-n]   the real tunnel + the lan front on :18950 (start.sh); the token comes from
#                                     $OC1_TOKEN_FILE (a test workspace's oc1 token, 0600, never printed)
# Extra env for the app: SIM_ENV="OPTCHAT_LAN_PINGER=250 ..." (each passed as SIMCTL_CHILD_...).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:-fake}"; N="${2:-20}"
SIM="${SIM_UDID:-$(xcrun simctl list devices | grep 'OC LAN iPhone 18 Pro' | grep -oE '[0-9A-F-]{36}' | head -1)}"
APP="$ROOT/target/xcode-lan/Build/Products/Debug-iphonesimulator/OptChat.app"
BID=com.reframe.optchat.lantest
LAN="${LAN_DIR:-$HOME/Work/lan-tunnel-poc/native-lan}"
LOG="$(mktemp -d)"; PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done; pkill -f "dns-sd -R OptChat LAN.*18961" 2>/dev/null; true; }
trap cleanup EXIT
if [ "$MODE" = fake ]; then
  deno run -A "$ROOT/tests/lib/fake-engine.ts" 18791 alpha > "$LOG/engine.log" 2>&1 & PIDS+=($!)
  mkdir -p "$LOG/sidecar"; ln -s "$LAN/sidecar/certs" "$LOG/sidecar/certs"
  sed 's/target.protocol = "wss:";/target.protocol = UPSTREAM.protocol === "http:" ? "ws:" : "wss:";/' "$LAN/sidecar/sidecar.ts" > "$LOG/sidecar/sidecar.ts"
  ( cd "$LOG" && LAN_PORT=18961 LAN_UPSTREAM=http://127.0.0.1:18791 LAN_ENGINE=127.0.0.1 exec deno run --no-lock --allow-net \
      --allow-env --allow-read --allow-run=dns-sd,scutil --allow-sys=networkInterfaces,hostname sidecar/sidecar.ts ) > "$LOG/sidecar.log" 2>&1 & PIDS+=($!)
  echo "${PIDS[1]}" > "$LOG/sidecar.pid"
  HDR=$(printf '{"v":1,"urls":["http://127.0.0.1:18791"],"id":"t1"}' | base64 | tr '+/' '-_' | tr -d '=')
  TOKEN="oc1.$HDR.s3cret"
else
  [ -f "${OC1_TOKEN_FILE:-}" ] || { echo "OC1_TOKEN_FILE (a test workspace's oc1 token) is needed"; exit 2; }
  TOKEN="$(cat "$OC1_TOKEN_FILE")"
fi
sleep 1
xcrun simctl boot "$SIM" 2>/dev/null; xcrun simctl bootstatus "$SIM" >/dev/null
xcrun simctl install "$SIM" "$APP"
DATA="$(xcrun simctl get_app_container "$SIM" $BID data)"; mkdir -p "$DATA/Documents"; : > "$DATA/Documents/diag.log"
ENVS=(SIMCTL_CHILD_OPTCHAT_RESET=1 "SIMCTL_CHILD_OPTCHAT_SEED_OC1=$(printf %s "$TOKEN" | base64)" "SIMCTL_CHILD_OPTCHAT_LAN_BENCH=$N")
# real: the tunnel, even though this Mac's own address (chat.reframe.dev) answers in the simulator
[ "$MODE" = real ] && ENVS+=(SIMCTL_CHILD_OPTCHAT_NO_LOCAL_URL=1)
for e in ${SIM_ENV:-}; do ENVS+=("SIMCTL_CHILD_$e"); done
xcrun simctl terminate "$SIM" $BID 2>/dev/null
env "${ENVS[@]}" xcrun simctl launch "$SIM" $BID > /dev/null
echo "launched; log: $DATA/Documents/diag.log  (servers: $LOG)"
for i in $(seq 1 ${SIM_WAIT:-120}); do grep -q "lan-bench" "$DATA/Documents/diag.log" 2>/dev/null && break; sleep 1; done
grep -E "lan:" "$DATA/Documents/diag.log"
[ -n "${SIM_HOLD:-}" ] && { echo "holding ${SIM_HOLD}s (sidecar pid $(cat "$LOG/sidecar.pid" 2>/dev/null))"; sleep "$SIM_HOLD"; grep -E "lan:" "$DATA/Documents/diag.log" | tail -30; }
xcrun simctl terminate "$SIM" $BID 2>/dev/null; true
