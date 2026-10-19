#!/bin/bash
# wp83 (on the VM): login83.sh base|new [packs]    how long a login takes (the bots' connect-to-join latency, p50/p99) and how long the server
#   takes to start, against the build of main (`base`) or of this branch (`new`), optionally with test packs (tools/wp83/packs) in the world.
#   20 idle bots join at 20 a second, three rounds in a row on the same server; Kiln on port 25589; noise terrain.
which=$1; packs=${2:-}
case $which in base) dir=$HOME/wt/wp83-base;; new) dir=$HOME/wt/wp83-datapacks;; *) echo "base|new [packs]"; exit 2;; esac
D=/tmp/wp83-login-$which; rm -rf $D; mkdir -p $D/world/datapacks
for p in $packs; do cp -r "$HOME/wt/wp83-datapacks/tools/wp83/packs/$p" "$D/world/datapacks/$p"; done
cd $D
rm -f console; mkfifo console
sleep 100000000 > console 2>/dev/null < /dev/null &
keeper=$!
export KILN_VIEW_DISTANCE=4 KILN_SIMULATION_DISTANCE=4 KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=83 KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORLD=$D/world KILN_WORK=$HOME/kiln/work RUST_LOG=info
start=$(date +%s.%N)
$dir/target/release/kiln < console > server.log 2>&1 &
pid=$!
for i in $(seq 1 200); do grep -q "listening on" server.log 2>/dev/null && break; sleep 0.1; done
up=$(date +%s.%N)
for i in $(seq 1 100); do grep -q "tick pool" server.log 2>/dev/null && break; sleep 0.1; done
ready=$(date +%s.%N)
echo "start: listening after $(echo "$up - $start" | bc) s, ready after $(echo "$ready - $start" | bc) s"
for round in 1 2 3; do
  $HOME/wt/wp83-datapacks/target/release/kiln-bot --addr 127.0.0.1:25589 --count 20 --rate 20 --duration 8 --behavior idle --name-prefix R${round}B 2>&1 | grep -iE "join" | tail -2
done
echo stop > console
for i in $(seq 1 100); do kill -0 $pid 2>/dev/null || break; sleep 0.2; done
kill $keeper 2>/dev/null
rm -rf $D
