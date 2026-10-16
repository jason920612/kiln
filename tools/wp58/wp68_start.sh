#!/bin/bash
# wp68 server on the VM: ~/wp68/world, port 25589, console through the ~/wp68/console fifo.
# usage: wp68_start.sh [binary]   (default: the wp68-playthrough6 checkout's release build)
mkdir -p ~/wp68 && cd ~/wp68
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run68 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp68-playthrough6/target/release/kiln}" ~/wp68/kiln-run68
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-68} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp68/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run68 < console > server.log 2>&1 &
echo started $!
