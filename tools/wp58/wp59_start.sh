#!/bin/bash
# wp59 server on the VM: ~/wp59/world, port 25585, console through the ~/wp59/console fifo.
# usage: wp59_start.sh [binary]   (default: the wp59-playthrough2 checkout's release build)
mkdir -p ~/wp59 && cd ~/wp59
pkill -f "wp59/kiln-run" 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp59-playthrough2/target/release/kiln}" ~/wp59/kiln-run
[ -p console ] || mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25585 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-59} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=$HOME/wp59/world KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
