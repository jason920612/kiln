#!/bin/bash
# wp72: restart the vanilla reference server with its world (stop it first, then start again; wp72_vanilla.sh deletes the world)
cd ~/wp72/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); fi
for i in $(seq 1 40); do pgrep -f "wp72vanilla-marker" > /dev/null || break; sleep 1; done
[ "$1" = stop ] && exit 0
rm -f console; mkfifo console
(sleep 100000000 > console &)
nohup java -Xmx2G -Dwp72vanilla-marker=1 -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
echo started $!
