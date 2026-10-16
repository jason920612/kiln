#!/bin/bash
# wp73 servers on the VM: Kiln (port 25590, ~/wp73/K) or vanilla 26.3 (port 25597, ~/wp73/V), a fresh noise world
# each start (seed $SEED, default 70), view and simulation distance 10, console through the fifo <dir>/console,
# log <dir>/server.log.
# usage: servers73.sh K|V start|stop [kiln binary]
which="$1"; action="$2"
dir="$HOME/wp73/$which"
seed="${SEED:-70}"
mkdir -p "$dir" && cd "$dir" || exit 1
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 5; fi
if [ "$which" = K ]; then pkill -x kiln-run73 2>/dev/null; else pkill -f "wp73vanilla-marker" 2>/dev/null; fi
sleep 1
[ "$action" = stop ] && exit 0
rm -rf world
rm -f console; mkfifo console
(sleep 100000000 > console &)
if [ "$which" = K ]; then
  cp "${3:-$HOME/wt/wp73-stalls/target/release/kiln}" kiln-run73
  export KILN_PORT=25590 KILN_GENERATOR=noise KILN_SEED=$seed KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORLD="$dir/world" \
    KILN_LANG=/home/test/kiln/work/wp2-commands/diff/en_us.json KILN_WORK=$HOME/kiln/work \
    KILN_VIEW_DISTANCE=10 KILN_SIMULATION_DISTANCE=10 RUST_LOG=info NO_COLOR=1
  nohup ./kiln-run73 < console > server.log 2>&1 &
else
  cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
  echo "eula=true" > eula.txt
  cat > server.properties <<PROP
server-port=25597
online-mode=false
level-type=minecraft:normal
level-seed=$seed
gamemode=creative
difficulty=normal
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
  nohup java -Xmx2G -Dwp73vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
fi
echo started $!
