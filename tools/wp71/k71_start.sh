#!/bin/bash
# wp71: a Kiln server on the VM (port 25599, fresh void world with the wp66 datapack, console fifo ~/wp71/console, log ~/wp71/server.log).
# usage: k71_start.sh [stop]; env KILN_BIN (default ~/wt/wp71-farm-rng/target/release/kiln), KILN_GENERATOR (noise: vanilla terrain; unset: void), KILN_SEED
mkdir -p ~/wp71 && cd ~/wp71
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run71 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
cp "${KILN_BIN:-$HOME/wt/wp71-farm-rng/target/release/kiln}" ~/wp71/kiln-run71
rm -rf ~/wp71/world; mkdir -p ~/wp71/world/datapacks && cp -r "${DP:-$HOME/wt/wp71-farm-rng/tools/datapacks/wp66}" ~/wp71/world/datapacks/wp66
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25599 KILN_SEED=${KILN_SEED:-65} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=$HOME/wp71/world KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
[ -n "$KILN_GENERATOR" ] && export KILN_GENERATOR
nohup ./kiln-run71 < console > server.log 2>&1 &
echo started $!
