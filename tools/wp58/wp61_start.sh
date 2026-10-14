#!/bin/bash
# wp61 server on the VM: ~/wp61/world, port 25585, console through the ~/wp61/console fifo.
# usage: wp61_start.sh [binary]   (default: the wp61-playthrough4 checkout's release build)
mkdir -p ~/wp61 && cd ~/wp61
[ -p console ] && echo stop > console; sleep 3; pkill -x kiln-run 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp61-playthrough4/target/release/kiln}" ~/wp61/kiln-run
[ -p console ] || mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25585 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-61} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp61/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
