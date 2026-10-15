#!/bin/bash
# wp66: a vanilla 26.3 reference server on the VM (port 25594, offline, console fifo ~/wp66/vanilla/console, log server.log).
# usage: wp66_vanilla.sh [stop]   env: VSEED (65), VTYPE (minecraft:normal | minecraft:flat), KEEP=1 keeps the world
mkdir -p ~/wp66/vanilla && cd ~/wp66/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 4; fi
pkill -f "wp66vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
[ -z "$KEEP" ] && rm -rf world
cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
echo "eula=true" > eula.txt
cat > server.properties <<PROP
server-port=25594
online-mode=false
level-type=${VTYPE:-minecraft:normal}
gamemode=creative
difficulty=normal
spawn-protection=0
view-distance=8
simulation-distance=8
max-tick-time=-1
sync-chunk-writes=false
enable-command-block=true
generate-structures=true
pause-when-empty-seconds=-1
white-list=false
level-seed=${VSEED:-65}
PROP
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx2G -Dwp66vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
