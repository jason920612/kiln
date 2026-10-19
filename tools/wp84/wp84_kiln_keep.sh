#!/bin/bash
# wp84: stop the Kiln server gracefully and start it again on the same world (same binary)
cd ~/wp84
if [ -p console ]; then (exec 3<>console; echo stop >&3); fi
for i in $(seq 1 40); do pgrep -x kiln-run84 > /dev/null || break; sleep 1; done
[ "$1" = stop ] && exit 0
rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=${SIMDIST:-8} KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-84} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=${KILN_WORLD:-$HOME/wp84/world} KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work RUST_LOG=info
nohup ./kiln-run84 < console > server.log 2>&1 &
echo started $!
