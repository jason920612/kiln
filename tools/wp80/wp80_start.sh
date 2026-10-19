#!/bin/bash
# wp80 server on the VM: ~/wp80/world, port 25589, console through the ~/wp80/console fifo.
# usage: wp80_start.sh [binary]   (default: the wp80 checkout)
mkdir -p ~/wp80 && cd ~/wp80
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run80 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp80-playthrough10/target/release/kiln}" ~/wp80/kiln-run80
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=${SIMDIST:-8} KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-80} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp80/world} KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run80 < console > server.log 2>&1 &
echo started $!
