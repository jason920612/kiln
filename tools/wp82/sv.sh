#!/bin/bash
# wp82 (on the VM): sv.sh K|V start|stop|fresh [simdist]   Kiln (port 25599, ~/wp82/K) or the vanilla reference server (25596, ~/wp82/V).
#   start: keep the world; fresh: delete it first; both take the console from the fifo ~/wp82/<K|V>/console and log to server.log.
#   SEED (default 77), SIMDIST (default 6), VIEWDIST (default 8), AGENT=1 starts vanilla with the spawn-counter java agent (W72_OUT=~/wp82/V/agent.out, W72_BOX).
#   Processes are tracked by PID files (server.pid, keeper.pid) -- nothing is stopped by a pattern.
who=$1; cmd=$2; sim=${3:-${SIMDIST:-6}}
seed=${SEED:-77}; view=${VIEWDIST:-8}
D=$HOME/wp82/$who
mkdir -p $D
stop() {
  if [ -f $D/server.pid ] && kill -0 $(cat $D/server.pid) 2>/dev/null; then
    (exec 3<>$D/console; echo stop >&3)
    for i in $(seq 1 60); do kill -0 $(cat $D/server.pid) 2>/dev/null || break; sleep 1; done
    kill -0 $(cat $D/server.pid) 2>/dev/null && kill $(cat $D/server.pid)
  fi
  [ -f $D/keeper.pid ] && kill $(cat $D/keeper.pid) 2>/dev/null
  rm -f $D/server.pid $D/keeper.pid
  true
}
if [ $cmd = stop ]; then stop; exit 0; fi
stop
if [ $cmd = fresh ]; then rm -rf $D/world $D/trace.out; fi
cd $D
rm -f console; mkfifo console
sleep 100000000 > console 2>/dev/null </dev/null &
echo $! > keeper.pid
if [ $who = K ]; then
  cp -f $HOME/wt/wp82-gaps/target/release/kiln $D/kiln-run82
  export KILN_VIEW_DISTANCE=$view KILN_SIMULATION_DISTANCE=$sim KILN_PORT=25599 KILN_GENERATOR=noise KILN_SEED=$seed KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORLD=$D/world KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=${OPS:-W0} KILN_WORK=$HOME/kiln/work RUST_LOG=info
  [ -n "$COUNTERS" ] && export KILN_SPAWN_TRACE=$D/trace.out KILN_SPAWN_COUNTERS=$D/counters.out KILN_SPAWN_BOX=${BOX:-0,0,0,0,0,0}
  nohup $D/kiln-run82 < console > server.log 2>&1 &
  echo $! > server.pid
else
  [ -f eula.txt ] || echo eula=true > eula.txt
  cat > server.properties <<PROPS
difficulty=${DIFFICULTY:-normal}
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
white-list=false
enforce-whitelist=false
allow-flight=true
PROPS
  W=$HOME/kiln/work
  if [ -n "$W72" ]; then
    # the wp72 spawn-counter agent (tools/wp82/agent72/build.sh -> ~/wp82/agent72.jar); AGENT=1 W72=1
    CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
    W72_OUT=$D/agent.out nohup java -Xmx1g -Dw72.box=${BOX:-0,0,0,0,0,0} -Dw72.trace=$D/trace.out -javaagent:$HOME/wp82/agent72.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
  elif [ -n "$AGENT" ]; then
    CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
    W82_OUT=$D/agent.out nohup java -Xmx1g -javaagent:$HOME/wp82/agent82.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
  else
    nohup java -Xmx1g -jar $W/server.jar nogui < console > server.log 2>&1 &
  fi
  echo $! > server.pid
fi
echo started $who $(cat server.pid)
