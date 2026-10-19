#!/bin/bash
# wp85 (on the VM): sv85.sh K|V start|stop|fresh   Kiln (port 25599, ~/wp85/K) or the vanilla reference server (25596, ~/wp85/V).
#   start: keep the world; fresh: delete it first (the packs in PACKS are copied into world/datapacks before the first start);
#   both take the console from the fifo ~/wp85/<K|V>/console and log to server.log. Processes are stopped by their pid files.
#   PACKS  (default "")  test packs of tools/wp85/packs to put in the world; PACKROOT (default ~/wt/wp85-worldgen-packs/tools/wp85/packs)
#   SEED (default 85), SIMDIST (default 6), VIEWDIST (default 8), BIN (Kiln binary, default ~/wt/wp85-worldgen-packs/target/release/kiln)
#   NAME (default = who): the directory ~/wp85/<NAME> (several worlds side by side; ports stay the same, run one server at a time).
who=$1; cmd=$2
seed=${SEED:-85}; view=${VIEWDIST:-8}; sim=${SIMDIST:-6}
D=$HOME/wp85/${NAME:-$who}
PACKS=${PACKS-}
PACKROOT=${PACKROOT:-$HOME/wt/wp85-worldgen-packs/tools/wp85/packs}
BIN=${BIN:-$HOME/wt/wp85-worldgen-packs/target/release/kiln}
mkdir -p $D
stop() {
  if [ -p $D/console ]; then (exec 3<>$D/console; echo stop >&3) 2>/dev/null; fi
  if [ -f $D/server.pid ]; then
    pid=$(cat $D/server.pid)
    for i in $(seq 1 90); do kill -0 $pid 2>/dev/null || break; sleep 1; done
    kill -0 $pid 2>/dev/null && kill $pid 2>/dev/null
    rm -f $D/server.pid
  fi
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
  cp -f $BIN $D/kiln-run85
  export KILN_VIEW_DISTANCE=$view KILN_SIMULATION_DISTANCE=$sim KILN_PORT=25599 KILN_GENERATOR=noise KILN_SEED=$seed KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORLD=$D/world KILN_ENABLE_COMMAND_BLOCK=true KILN_OPS=${OPS:-W0} KILN_WORK=$HOME/kiln/work RUST_LOG=${RUST_LOG:-info}
  setsid bash -c 'echo $$ > "$0"; exec "$1"' $D/server.pid $D/kiln-run85 < console > server.log 2>&1 &
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
white-list=false
enforce-whitelist=false
PROPS
  W=$HOME/kiln/work
  setsid bash -c 'echo $$ > "$0"; exec java -Xmx2G -jar "$1" nogui' $D/server.pid $W/server.jar < console > server.log 2>&1 &
fi
echo started $who
