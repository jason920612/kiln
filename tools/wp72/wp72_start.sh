#!/bin/bash
# wp72 server on the VM: ~/wp72/world, port 25589, console through the ~/wp72/console fifo.
# usage: wp72_start.sh [binary]   (default: the wp72-playthrough7 checkout's release build)
mkdir -p ~/wp72 && cd ~/wp72
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run72 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp72-playthrough7/target/release/kiln}" ~/wp72/kiln-run72
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=8 KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-72} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp72/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run72 < console > server.log 2>&1 &
echo started $!
