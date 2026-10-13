#!/bin/bash
# wp60 server on the VM: ~/wp60/world, port 25585, console through the ~/wp60/console fifo.
# usage: wp60_start.sh [binary]   (default: the wp60-playthrough3 checkout's release build)
mkdir -p ~/wp60 && cd ~/wp60
pkill -f "wp60/kiln-run" 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp60-playthrough3/target/release/kiln}" ~/wp60/kiln-run
[ -p console ] || mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25585 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-59} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=$HOME/wp60/world KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
