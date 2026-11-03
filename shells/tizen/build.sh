#!/bin/sh
# build libocgpui.so for the tv, wrap it in the c# host tpk, sign with the samsung `optchat` profile, install + run.
#   --dev-capture   dev build: screenshots, test channel, fake mic, logs via the mac dev server (never ship it)
#   --seed FILE     dev build: FILE (json {engines:[{endpoint,name,ws,token}],active}) goes in as res/dev-seed.json;
#                   it holds tokens: kept 0600, git-ignored, removed from the tree after packaging
#   --dev-host IP   dev build: the mac running the dev servers (logs :8765, dev channel :8767, mic dump :8768)
#   --theme-preview temporary: cycle all themes every 15 s with the name on top (to pick one)
#   --no-install    stop after packaging
set -e
cd "$(dirname "$0")/../.."
. shells/tizen/env.sh
FEAT=""; SEED=""; DEVHOST=""; prev=""
for a in "$@"; do
  [ "$a" = "--dev-capture" ] && FEAT="$FEAT dev-capture"
  [ "$a" = "--theme-preview" ] && FEAT="$FEAT theme-preview"
  [ "$prev" = "--seed" ] && SEED="$a"
  [ "$prev" = "--dev-host" ] && DEVHOST="$a"
  prev="$a"
done
T=${TIZEN_STUDIO:-$HOME/tizen-studio}
P=${TIZEN_PROFILE:-optchat}   # your samsung certificate profile (certificate manager), never committed
mkdir -p target/tizen-stubs
[ -f target/tizen-stubs/libxkbcommon.so ] || $TCC -shared -o target/tizen-stubs/libxkbcommon.so shells/tizen/toolchain/xkb-stub.c
OCTV_DEV_HOST="$DEVHOST" cargo build -p optchat-tizen --target armv7-unknown-linux-gnueabi --release ${FEAT:+--features "$FEAT"}
$T/tools/llvm-10/bin/llvm-strip -o shells/tizen/host/lib/libocgpui.so target/armv7-unknown-linux-gnueabi/release/libocgpui.so
cd shells/tizen/host
S=shared/res/dev-seed.json
rm -f $S
if [ -n "$SEED" ]; then (umask 077; cp "$SEED" $S); fi
${DOTNET:-$HOME/.dotnet/dotnet} build -c Release -v q | grep -E 'error|tpk' || true
rm -f $S
TIZ=$T/tools/ide/bin/tizen
$TIZ package -t tpk -s $P -- bin/Release/tizen80/so.reframe.octv-0.0.1.tpk | tail -1
case " $* " in *" --no-install "*) rm -f bin/Release/tizen80/tpkroot/$S;; esac
case " $* " in *" --no-install "*) exit 0;; esac
$TIZ install -n so.reframe.octv-0.0.1.tpk -s ${TIZEN_SERIAL:-emulator-26111} -- bin/Release/tizen80 | tail -1
rm -f bin/Release/tizen80/tpkroot/$S
$TIZ run -p so.reframe.octv -s ${TIZEN_SERIAL:-emulator-26111} | tail -1
