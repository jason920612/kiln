#!/bin/bash
# wp84 server on the VM: ~/wp84/world, port 25589, console through the ~/wp84/console fifo.
# usage: wp84_start.sh [binary]   (default: the wp84 checkout)
mkdir -p ~/wp84 && cd ~/wp84
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run84 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp84-playthrough11/target/release/kiln}" ~/wp84/kiln-run84
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=${SIMDIST:-8} KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-84} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp84/world} KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run84 < console > server.log 2>&1 &
echo started $!
