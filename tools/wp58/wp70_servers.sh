#!/bin/bash
# wp70 command benchmark servers on the VM: Kiln (port 25590, ~/wp70/K) or vanilla 26.3 (port 25597, ~/wp70/V),
# a fresh noise world (seed 70) each start, the wp70 bench datapack (~/wp70/pack/wp70), console through the
# fifo <dir>/console, log <dir>/server.log.
# usage: wp70_servers.sh K|V start|stop [kiln binary]
which="$1"; action="$2"
dir="$HOME/wp70/$which"
mkdir -p "$dir" && cd "$dir" || exit 1
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 5; fi
if [ "$which" = K ]; then pkill -x kiln-run70 2>/dev/null; else pkill -f "wp70vanilla-marker" 2>/dev/null; fi
sleep 1
[ "$action" = stop ] && exit 0
rm -rf world packs
rm -f console; mkfifo console
(sleep 100000000 > console &)
if [ "$which" = K ]; then
  cp "${3:-$HOME/wt/wp70-command-perf/target/release/kiln}" kiln-run70
  mkdir -p packs && cp -r ~/wp70/pack/wp70 packs/
  export KILN_PORT=25590 KILN_GENERATOR=noise KILN_SEED=70 KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD="$dir/world" \
    KILN_DATAPACKS="$dir/packs" KILN_LANG=/home/test/kiln/work/wp2-commands/diff/en_us.json KILN_WORK=$HOME/kiln/work \
    KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=8 RUST_LOG=info NO_COLOR=1
  nohup ./kiln-run70 < console > server.log 2>&1 &
else
  mkdir -p world/datapacks && cp -r ~/wp70/pack/wp70 world/datapacks/
  cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
  echo "eula=true" > eula.txt
  cat > server.properties <<PROP
server-port=25597
online-mode=false
level-type=minecraft:normal
level-seed=70
gamemode=creative
difficulty=peaceful
spawn-protection=0
view-distance=8
simulation-distance=8
max-tick-time=-1
sync-chunk-writes=false
enable-command-block=true
generate-structures=true
pause-when-empty-seconds=-1
white-list=false
PROP
  nohup java -Xmx2G -Dwp70vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
fi
echo started $!
