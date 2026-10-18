#!/bin/bash
# wp78 server on the VM: ~/wp78/world, port 25589, console through the ~/wp78/console fifo.
# usage: wp78_start.sh [binary]   (default: the wp78-playthrough9 checkout's release build)
mkdir -p ~/wp78 && cd ~/wp78
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run78 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp78-playthrough9/target/release/kiln}" ~/wp78/kiln-run78
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=${SIMDIST:-8} KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-72} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp78/world} KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run78 < console > server.log 2>&1 &
echo started $!
