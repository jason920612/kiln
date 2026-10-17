#!/bin/bash
# wp76 item benchmark servers on the VM: Kiln (port 25590, ~/wp76/K) or vanilla 26.3 (port 25597, ~/wp76/V),
# a fresh noise world (seed 76) each start, the wp76 datapack (~/wp76/pack/wp76), console through the fifo
# <dir>/console, log <dir>/server.log. Simulation and view distance 10 (the user's defaults).
# usage: servers76.sh K|V start|stop [kiln binary]   (extra Kiln env: K76_ENV="A=1 B=2")
which="$1"; action="$2"
dir="$HOME/wp76/$which"
mkdir -p "$dir" && cd "$dir" || exit 1
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 5; fi
if [ "$which" = K ]; then pkill -x kiln-run76 2>/dev/null; else pkill -f "wp76vanilla-marker" 2>/dev/null; fi
sleep 1
[ "$action" = stop ] && exit 0
rm -rf world packs
rm -f console; mkfifo console
(setsid sleep 100000000 > console < /dev/null 2>&1 &)
if [ "$which" = K ]; then
  cp "${3:-$HOME/wt/wp76-items-perf/target/release/kiln}" kiln-run76
  mkdir -p world/datapacks && cp -r ~/wp76/pack/wp76 world/datapacks/
  export KILN_PORT=25590 KILN_GENERATOR=noise KILN_SEED=76 KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD="$dir/world" \
    KILN_LANG=/home/test/kiln/work/wp2-commands/diff/en_us.json KILN_WORK=$HOME/kiln/work \
    KILN_VIEW_DISTANCE=10 KILN_SIMULATION_DISTANCE=10 RUST_LOG=info NO_COLOR=1 KILN_TICK_TRACE="$dir/trace.txt"
  for kv in $K76_ENV; do export "$kv"; done
  setsid nohup ./kiln-run76 < console > server.log 2>&1 &
else
  mkdir -p world/datapacks && cp -r ~/wp76/pack/wp76 world/datapacks/
  cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
  echo "eula=true" > eula.txt
  cat > server.properties <<PROP
server-port=25597
online-mode=false
level-type=minecraft:normal
level-seed=76
gamemode=creative
difficulty=peaceful
spawn-protection=0
view-distance=10
simulation-distance=10
max-tick-time=-1
sync-chunk-writes=false
enable-command-block=true
generate-structures=true
pause-when-empty-seconds=-1
white-list=false
PROP
  setsid nohup java -Xmx3G -Dwp76vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
fi
echo started $!
