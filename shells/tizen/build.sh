#!/bin/sh
# build libocgpui.so for the tv, wrap it in the c# host tpk, sign with the samsung `optchat` profile, install + run.
set -e
cd "$(dirname "$0")/../.."
. shells/tizen/env.sh
cargo build -p optchat-tizen --target armv7-unknown-linux-gnueabi --release
$HOME/tizen-studio/tools/llvm-10/bin/llvm-strip -o shells/tizen/host/lib/libocgpui.so target/armv7-unknown-linux-gnueabi/release/libocgpui.so
cd shells/tizen/host
$HOME/.dotnet/dotnet build -c Release -v q | grep -E 'error|tpk' || true
T=$HOME/tizen-studio/tools/ide/bin/tizen
$T package -t tpk -s optchat -- bin/Release/tizen80/so.reframe.octv-0.0.1.tpk | tail -1
[ "$1" = "--no-install" ] && exit 0
$T install -n so.reframe.octv-0.0.1.tpk -s emulator-26111 -- bin/Release/tizen80 | tail -1
$T run -p so.reframe.octv -s emulator-26111 | tail -1
