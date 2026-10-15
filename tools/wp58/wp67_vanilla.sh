#!/bin/bash
# wp67: the vanilla 26.3 server as a reference for the real-client elytra flight: port 25597 (not at the same time as
# wp67_start.sh), seed 65, view and simulation distance 10, survival, console through the fifo ~/wp67/rc-v/console,
# log ~/wp67/rc-v/server.log. usage: wp67_vanilla.sh [stop]
mkdir -p ~/wp67/rc-v && cd ~/wp67/rc-v
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 8; fi
pkill -f "wp67vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
rm -rf world
cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
echo "eula=true" > eula.txt
cat > server.properties <<PROP
server-port=25597
online-mode=false
level-type=minecraft:normal
gamemode=survival
difficulty=normal
spawn-protection=0
view-distance=10
simulation-distance=10
max-tick-time=-1
sync-chunk-writes=false
generate-structures=true
pause-when-empty-seconds=-1
white-list=false
level-seed=${VSEED:-65}
PROP
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx3G -Dwp67vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
