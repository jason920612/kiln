#!/bin/bash
# wp78: a vanilla 26.3 reference server on the VM for side-by-side checks (port 25595, offline, flat world,
# console through the fifo ~/wp78/vanilla/console, log ~/wp78/vanilla/server.log).
# usage: wp78_vanilla.sh [stop]
mkdir -p ~/wp78/vanilla && cd ~/wp78/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); sleep 4; fi
pkill -f "wp78vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
rm -rf world
cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
echo "eula=true" > eula.txt
cat > server.properties <<PROP
server-port=25595
online-mode=false
level-type=minecraft:normal
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
level-seed=${VSEED:-72}
PROP
# a fresh fifo: a stop nobody read must not stop the new server
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx2G -Dwp78vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
