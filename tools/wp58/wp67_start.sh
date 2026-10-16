#!/bin/bash
# wp67 server on the VM for the real-client elytra flight: a fresh world ~/wp67/rc/world (seed 65 like wp65), port 25597,
# console through the ~/wp67/rc/console fifo, per-tick trace ~/wp67/rc/ticks.
# usage: wp67_start.sh <binary> | stop
mkdir -p ~/wp67/rc && cd ~/wp67/rc
[ -p console ] && (exec 3<>console; echo stop >&3); sleep 5; pkill -x kiln-rc67 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
cp "$1" ~/wp67/rc/kiln-rc67
rm -rf world ticks; rm -f console; mkfifo console
(sleep 100000000 > console &)
export KILN_PORT=25597 KILN_GENERATOR=noise KILN_SEED=${KILN_SEED:-65} KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD=$HOME/wp67/rc/world \
  KILN_OPS=KilnTest KILN_WORK=$HOME/kiln/work KILN_TICK_TRACE=$HOME/wp67/rc/ticks KILN_SLOW_PRINT=${KILN_SLOW_PRINT:-10} RUST_LOG=info
nohup ./kiln-rc67 < console > server.log 2>&1 &
echo started $!
