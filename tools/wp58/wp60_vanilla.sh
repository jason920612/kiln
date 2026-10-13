#!/bin/bash
# wp60: a vanilla 26.3 reference server on the VM for side-by-side checks (port 25591, offline, flat world,
# console through the fifo ~/wp60/vanilla/console, log ~/wp60/vanilla/server.log).
# usage: wp60_vanilla.sh [stop]
mkdir -p ~/wp60/vanilla && cd ~/wp60/vanilla
if [ -p console ]; then echo stop > console; sleep 4; fi
pkill -f "wp60vanilla-marker" 2>/dev/null
[ "$1" = stop ] && exit 0
rm -rf world
cp -r ~/kiln/work/vanilla-check/libraries ~/kiln/work/vanilla-check/versions . 2>/dev/null
echo "eula=true" > eula.txt
cat > server.properties <<PROP
server-port=25591
online-mode=false
level-type=minecraft\:flat
gamemode=creative
difficulty=normal
spawn-protection=0
view-distance=8
simulation-distance=8
max-tick-time=-1
sync-chunk-writes=false
enable-command-block=true
generate-structures=false
pause-when-empty-seconds=-1
level-seed=60
PROP
[ -p console ] || mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx2G -Dwp60vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
