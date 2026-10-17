#!/bin/bash
# wp77 (on the VM): sv.sh K|V start|stop|fresh [simdist]   Kiln (port 25599, ~/wp77/K) or the vanilla reference server (25596, ~/wp77/V).
#   start: keep the world; fresh: delete it first; both take the console from the fifo ~/wp77/<K|V>/console and log to server.log.
#   SEED (default 77), SIMDIST (default 6), VIEWDIST (default 8), AGENT=1 starts vanilla with the spawn-counter java agent (W72_OUT=~/wp77/V/agent.out, W72_BOX).
who=$1; cmd=$2; sim=${3:-${SIMDIST:-6}}
seed=${SEED:-77}; view=${VIEWDIST:-8}
D=$HOME/wp77/$who
mkdir -p $D
stop() {
  if [ -p $D/console ]; then (exec 3<>$D/console; echo stop >&3); fi
  for i in $(seq 1 60); do
    if [ $who = K ]; then pgrep -f "$D/kiln-run77" > /dev/null || break; else pgrep -f "wp77vanilla-marker" > /dev/null || break; fi
    sleep 1
  done
  pkill -f "sleep 100000000 > $D/console" 2>/dev/null
  true
}
if [ $cmd = stop ]; then stop; exit 0; fi
stop
if [ $cmd = fresh ]; then rm -rf $D/world; fi
cd $D
rm -f console; mkfifo console
(sleep 100000000 > console &)
if [ $who = K ]; then
  cp -f $HOME/wt/wp77-gaps/target/release/kiln $D/kiln-run77
  export KILN_VIEW_DISTANCE=$view KILN_SIMULATION_DISTANCE=$sim KILN_PORT=25599 KILN_GENERATOR=noise KILN_SEED=$seed KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORLD=$D/world KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=${OPS:-W0} KILN_WORK=$HOME/kiln/work RUST_LOG=info
  [ -n "$COUNTERS" ] && export KILN_SPAWN_COUNTERS=$D/counters.out KILN_SPAWN_BOX=${BOX:-0,0,0,0,0,0}
  nohup $D/kiln-run77 < console > server.log 2>&1 &
else
  [ -f eula.txt ] || echo eula=true > eula.txt
  cat > server.properties <<PROPS
difficulty=normal
enable-command-block=true
gamemode=creative
generate-structures=true
level-name=world
level-seed=$seed
level-type=minecraft\:normal
max-tick-time=-1
online-mode=false
pause-when-empty-seconds=-1
server-port=25596
simulation-distance=$sim
spawn-protection=0
sync-chunk-writes=false
view-distance=$view
enforce-secure-profile=false
PROPS
  W=$HOME/kiln/work
  if [ -n "$AGENT" ]; then
    CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
    W72_OUT=$D/agent.out nohup java -Xmx2G -Dwp77vanilla-marker=1 -Dw72.box=${BOX:-0,0,0,0,0,0} -javaagent:$HOME/wp77/agent77.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
  else
    nohup java -Xmx2G -Dwp77vanilla-marker=1 -jar $W/server.jar nogui < console > server.log 2>&1 &
  fi
fi
echo started $who
