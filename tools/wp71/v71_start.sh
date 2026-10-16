#!/bin/bash
# wp71: vanilla 26.3 server on the VM (port 25596, own dir ~/wp71/vanilla, console fifo, log server.log) started from the class path (not the bundler) with the
# wp71 java agent (tools/wp71): per-tick villager statistics in $W71_OUT. usage: v71_start.sh [stop]
mkdir -p ~/wp71/vanilla && cd ~/wp71/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 5; fi
pkill -f "wp71vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
rm -rf world
echo "eula=true" > eula.txt
cat > server.properties <<PROP
server-port=25596
online-mode=false
level-type=minecraft:flat
gamemode=creative
difficulty=normal
spawn-protection=0
view-distance=${SIM:-8}
simulation-distance=${SIM:-8}
max-tick-time=-1
sync-chunk-writes=false
enable-command-block=true
generate-structures=false
pause-when-empty-seconds=-1
white-list=false
level-seed=65
PROP
mkdir -p world/datapacks && cp -r "${DP:-$HOME/wt/wp71-farm-rng/tools/datapacks/wp66}" world/datapacks/wp66
rm -f console; mkfifo console
(sleep 100000000 > console &)
W=$HOME/kiln/work
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
rm -f ${W71_OUT:-w71agent.out}
W71_OUT=${W71_OUT:-$PWD/w71agent.out} nohup java -Xmx1500M -Dwp71vanilla-marker=1 -Dw71.x0=4000 -Dw71.z0=4000 $W71_JOPTS -javaagent:$HOME/w71/agent.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
echo started $!
