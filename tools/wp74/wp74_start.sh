#!/bin/bash
# wp74 server on the VM: ~/wp74/world, port 25589, console through the ~/wp74/console fifo.
# usage: wp74_start.sh [binary]   (default: the wp74-playthrough8 checkout's release build)
mkdir -p ~/wp74 && cd ~/wp74
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run74 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp74-playthrough8/target/release/kiln}" ~/wp74/kiln-run74
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=${SIMDIST:-8} KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-72} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp74/world} KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run74 < console > server.log 2>&1 &
echo started $!
