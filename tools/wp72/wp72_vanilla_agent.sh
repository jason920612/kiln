#!/bin/bash
# wp72: restart the vanilla reference server (same world, port 25595) from the class path with the W72 java agent: slime spawn counters in $W72_OUT (~/wp72/slime_agent.out)
cd ~/wp72/vanilla
if [ -p console ]; then (exec 3<>console; echo stop >&3); fi
for i in $(seq 1 40); do pgrep -f "wp72vanilla-marker" > /dev/null || break; sleep 1; done
[ "$1" = stop ] && exit 0
rm -f console; mkfifo console
(sleep 100000000 > console &)
W=$HOME/kiln/work
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
W72_OUT=$HOME/wp72/slime_agent.out nohup java -Xmx2G -Dwp72vanilla-marker=1 -javaagent:$HOME/wp72/agent72.jar --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" net.minecraft.server.Main --nogui < console > server.log 2>&1 &
echo started $!
