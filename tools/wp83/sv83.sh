#!/bin/bash
# wp83 (on the VM): sv83.sh K|V start|stop|fresh   Kiln (port 25589, ~/wp83/K) or the vanilla reference server (25595, ~/wp83/V).
#   start: keep the world; fresh: delete it first (the packs in PACKS are copied into world/datapacks before the first start);
#   both take the console from the fifo ~/wp83/<K|V>/console and log to server.log.
#   PACKS  (default "kiln_wp83")  test packs of tools/wp83/packs to put in the world; PACKROOT (default ~/wt/wp83-datapacks/tools/wp83/packs)
#   SEED (default 83), SIMDIST (default 6), VIEWDIST (default 8), BIN (Kiln binary, default ~/wt/wp83-datapacks/target/release/kiln)
#   AGENT=1 starts vanilla with the packet-dump java agent of tools/wp74/agent (output ~/wp83/V/agent.out).
who=$1; cmd=$2
seed=${SEED:-83}; view=${VIEWDIST:-8}; sim=${SIMDIST:-6}
D=$HOME/wp83/$who
PACKS=${PACKS-kiln_wp83}
PACKROOT=${PACKROOT:-$HOME/wt/wp83-datapacks/tools/wp83/packs}
BIN=${BIN:-$HOME/wt/wp83-datapacks/target/release/kiln}
mkdir -p $D
stop() {
  if [ -p $D/console ]; then (exec 3<>$D/console; echo stop >&3); fi
  for i in $(seq 1 60); do
    if [ $who = K ]; then pgrep -f "$D/kiln-run83" > /dev/null || break; else pgrep -f "wp83vanilla-marker" > /dev/null || break; fi
    sleep 1
  done
  if [ -f $D/keeper.pid ]; then kill $(cat $D/keeper.pid) 2>/dev/null; rm -f $D/keeper.pid; fi
  true
}
if [ "$cmd" = stop ]; then stop; exit 0; fi
stop
if [ "$cmd" = fresh ]; then rm -rf $D/world $D/trace.out; fi
mkdir -p $D/world/datapacks
for p in $PACKS; do rm -rf $D/world/datapacks/$p; cp -r $PACKROOT/$p $D/world/datapacks/$p; done
cd $D
rm -f console; mkfifo console
sleep 100000000 > console 2>/dev/null < /dev/null &
echo $! > $D/keeper.pid
if [ $who = K ]; then
  cp -f $BIN $D/kiln-run83
  export KILN_VIEW_DISTANCE=$view KILN_SIMULATION_DISTANCE=$sim KILN_PORT=25589 KILN_GENERATOR=noise KILN_SEED=$seed KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORLD=$D/world KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=${OPS:-W0} KILN_WORK=$HOME/kiln/work RUST_LOG=${RUST_LOG:-info}
  setsid nohup $D/kiln-run83 < console > server.log 2>&1 &
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
server-port=25595
simulation-distance=$sim
spawn-protection=0
sync-chunk-writes=false
view-distance=$view
enforce-secure-profile=false
white-list=false
enforce-whitelist=false
PROPS
  W=$HOME/kiln/work
  if [ -n "$AGENT" ]; then
    CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
    setsid nohup java -Xmx2G -Dwp83vanilla-marker=1 -javaagent:${AGENTJAR:-$HOME/wp83/agent83.jar} --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
  else
    setsid nohup java -Xmx2G -Dwp83vanilla-marker=1 -jar $W/server.jar nogui < console > server.log 2>&1 &
  fi
fi
echo started $who
