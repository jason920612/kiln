#!/bin/bash
# wp64 server on the VM: ~/wp64/world, port 25586, console through the ~/wp64/console fifo.
# usage: wp64_start.sh [binary]   (default: the wp64-wp63-gaps checkout's release build)
mkdir -p ~/wp64 && cd ~/wp64
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp64-wp63-gaps/target/release/kiln}" ~/wp64/kiln-run
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25586 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-63} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp64/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run < console > server.log 2>&1 &
echo started $!
