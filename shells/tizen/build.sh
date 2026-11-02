#!/bin/sh
# build libocgpui.so for the tv, wrap it in the c# host tpk, sign with the samsung `optchat` profile, install + run.
#   --dev-capture   dev build: screenshots, test channel, fake mic, logs via the mac dev server (never ship it)
#   --seed FILE     dev build: FILE (json {engines:[{endpoint,name,ws,token}],active}) goes in as res/dev-seed.json;
#                   it holds tokens: kept 0600, git-ignored, removed from the tree after packaging
#   --no-install    stop after packaging
set -e
cd "$(dirname "$0")/../.."
. shells/tizen/env.sh
FEAT=""; SEED=""; prev=""
for a in "$@"; do
  [ "$a" = "--dev-capture" ] && FEAT="--features dev-capture"
  [ "$prev" = "--seed" ] && SEED="$a"
  prev="$a"
done
cargo build -p optchat-tizen --target armv7-unknown-linux-gnueabi --release $FEAT
$HOME/tizen-studio/tools/llvm-10/bin/llvm-strip -o shells/tizen/host/lib/libocgpui.so target/armv7-unknown-linux-gnueabi/release/libocgpui.so
cd shells/tizen/host
S=shared/res/dev-seed.json
rm -f $S
if [ -n "$SEED" ]; then (umask 077; cp "$SEED" $S); fi
$HOME/.dotnet/dotnet build -c Release -v q | grep -E 'error|tpk' || true
rm -f $S
T=$HOME/tizen-studio/tools/ide/bin/tizen
$T package -t tpk -s optchat -- bin/Release/tizen80/so.reframe.octv-0.0.1.tpk | tail -1
case " $* " in *" --no-install "*) rm -f bin/Release/tizen80/tpkroot/$S;; esac
case " $* " in *" --no-install "*) exit 0;; esac
$T install -n so.reframe.octv-0.0.1.tpk -s emulator-26111 -- bin/Release/tizen80 | tail -1
rm -f bin/Release/tizen80/tpkroot/$S
$T run -p so.reframe.octv -s emulator-26111 | tail -1
