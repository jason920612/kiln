#!/bin/bash
# wp84: restart the vanilla reference server with its world (stop it first, then start again; wp84_vanilla.sh deletes the world)
cd ~/wp84/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); fi
for i in $(seq 1 40); do pgrep -f "wp84vanilla-marker" > /dev/null || break; sleep 1; done
[ "$1" = stop ] && exit 0
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx2G -Dwp84vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
