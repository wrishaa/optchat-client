#!/bin/bash
# Lan-first live test on this Mac (crates/client/tests/lan_live.rs): a fake engine on 127.0.0.1:18791 as the
# "tunnel", the lan front poc (~/Work/lan-tunnel-poc/native-lan/sidecar) on :18961 in front of it, advertised with
# engine=127.0.0.1. A fake token only (the fake engine's); nothing real is touched. Stops both at the end.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LAN="${LAN_DIR:-$HOME/Work/lan-tunnel-poc/native-lan}"
LOG="$(mktemp -d)"
deno run -A "$ROOT/tests/lib/fake-engine.ts" 18791 alpha > "$LOG/engine.log" 2>&1 & EP=$!
# a copy of the front whose upstream websocket follows the upstream's scheme (the fake engine is plain http);
# the poc's own file is left as it is
mkdir -p "$LOG/sidecar"; ln -s "$LAN/sidecar/certs" "$LOG/sidecar/certs"
sed 's/target.protocol = "wss:";/target.protocol = UPSTREAM.protocol === "http:" ? "ws:" : "wss:";/' "$LAN/sidecar/sidecar.ts" > "$LOG/sidecar/sidecar.ts"
( cd "$LOG" && LAN_PORT=18961 LAN_UPSTREAM=http://127.0.0.1:18791 LAN_ENGINE=127.0.0.1 LAN_TICKET_TTL=600 \
    exec deno run --no-lock --allow-net --allow-env --allow-read --allow-run=dns-sd,scutil \
    --allow-sys=networkInterfaces,hostname sidecar/sidecar.ts ) > "$LOG/sidecar.log" 2>&1 & SP=$!
for i in $(seq 1 50); do curl -sk -o /dev/null https://127.0.0.1:18961/lan/ping && break; sleep 0.1; done
HDR=$(printf '{"v":1,"urls":["http://127.0.0.1:18791"],"id":"t1"}' | base64 | tr '+/' '-_' | tr -d '=')
cd "$ROOT"
LAN_SIDECAR_PID=$SP LAN_TEST_TOKEN="oc1.$HDR.s3cret" OPTCHAT_LAN_HB_MS=1000 OPTCHAT_LAN_RETRY_S=3 \
  cargo test -p optchat-client --test lan_live -- --ignored --nocapture; RC=$?
kill $EP 2>/dev/null; kill -CONT $SP 2>/dev/null; kill $SP 2>/dev/null
pkill -f "dns-sd -R OptChat LAN.*18961" 2>/dev/null
echo "logs: $LOG"; exit $RC
