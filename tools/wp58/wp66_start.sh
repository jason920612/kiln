#!/bin/bash
# wp66 Kiln server on the VM: ~/wp66/world (a fresh one unless KILN_WORLD), port 25588, console through the ~/wp66/console fifo.
# usage: wp66_start.sh [binary]   env: KILN_GENERATOR (noise|flat default), KILN_SEED, WORLD
mkdir -p ~/wp66 && cd ~/wp66
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run66 2>/dev/null
sleep 1
cp "${1:-$HOME/wt/wp66-wp65-gaps/target/release/kiln}" ~/wp66/kiln-run66
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25588 KILN_SEED=${KILN_SEED:-65} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${WORLD:-$HOME/wp66/world} KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
[ -n "$KILN_GENERATOR" ] && export KILN_GENERATOR
nohup ./kiln-run66 < console > server.log 2>&1 &
echo started $!
