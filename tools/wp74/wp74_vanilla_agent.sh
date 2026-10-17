#!/bin/bash
# wp74: restart the vanilla reference server (same world, port 25595) from the class path with the W72 java agent: slime spawn counters in $W72_OUT (~/wp74/slime_agent.out)
cd ~/wp74/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); fi
for i in $(seq 1 40); do pgrep -f "wp74vanilla-marker" > /dev/null || break; sleep 1; done
[ "$1" = stop ] && exit 0
rm -f console; mkfifo console
(sleep 100000000 > console &)
W=$HOME/kiln/work
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
W72_OUT=$HOME/wp74/slime_agent.out nohup java -Xmx2G -Dwp74vanilla-marker=1 -Dw72.box=${W72_BOX:-0,0,0,0,0,0} -javaagent:$HOME/wp74/agent74.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
echo started $!
