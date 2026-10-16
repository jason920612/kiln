#!/bin/bash
# wp69: a Kiln server on the VM (port 25599, fresh world with the wp66 datapack, console fifo ~/wp69/console, log ~/wp69/server.log).
# usage: k69_start.sh [stop] ; env KILN_BIN (default ~/wt/wp69-iron-farm/target/release/kiln), KILN_GENERATOR (noise|flat|empty: no generator = void), KILN_SEED
mkdir -p ~/wp69 && cd ~/wp69
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 3; pkill -x kiln-run69 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
cp "${KILN_BIN:-$HOME/wt/wp69-iron-farm/target/release/kiln}" ~/wp69/kiln-run69
rm -rf ~/wp69/world; mkdir -p ~/wp69/world/datapacks && cp -r "${DP:-$HOME/wt/wp69-iron-farm/tools/datapacks/wp66}" ~/wp69/world/datapacks/wp66
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25599 KILN_SEED=${KILN_SEED:-65} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=$HOME/wp69/world KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
[ -n "$KILN_GENERATOR" ] && export KILN_GENERATOR
nohup ./kiln-run69 < console > server.log 2>&1 &
echo started $!
