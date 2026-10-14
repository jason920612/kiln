#!/bin/bash
# wp65 server on the VM: ~/wp65/world, port 25587, console through the ~/wp65/console fifo.
# usage: wp65_start.sh [binary]   (default: the wp65-playthrough5 checkout's release build)
mkdir -p ~/wp65 && cd ~/wp65
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp65-playthrough5/target/release/kiln}" ~/wp65/kiln-run
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25587 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-65} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp65/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
