#!/bin/bash
# wp69: a vanilla 26.3 reference server on the VM (port 25596, offline, flat world, console fifo ~/wp69/vanilla/console, log server.log), with the wp66 datapack.
# usage: v69_start.sh [stop]
mkdir -p ~/wp69/vanilla && cd ~/wp69/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 4; fi
pkill -f "wp69vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
rm -rf world
cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
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
# VOID=1: nothing but a layer of bedrock (the world of the parity harness) instead of the flat world's dirt and grass
[ -n "$VOID" ] && echo 'generator-settings={"layers"\:[{"block"\:"minecraft\:bedrock","height"\:1}],"biome"\:"minecraft\:plains"}' >> server.properties
mkdir -p world/datapacks && cp -r "${DP:-$HOME/wt/wp69-iron-farm/tools/datapacks/wp66}" world/datapacks/wp66
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx1500M -Dwp69vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
