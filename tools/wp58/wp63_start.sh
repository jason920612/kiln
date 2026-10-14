#!/bin/bash
# wp63 server on the VM: ~/wp63/world, port 25585, console through the ~/wp63/console fifo.
# usage: wp63_start.sh [binary]   (default: the wp63-wp61-gaps checkout's release build)
mkdir -p ~/wp63 && cd ~/wp63
[ -p console ] && echo stop > console; sleep 3; pkill -x kiln-run 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp63-wp61-gaps/target/release/kiln}" ~/wp63/kiln-run
[ -p console ] || mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25585 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-63} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp63/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
