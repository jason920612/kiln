#!/bin/bash
# on the VM: restart both servers on their worlds (graceful stop) and reconnect the actor client (Kiln) and the second client (vanilla)
cd ~/wp84
bash wp84_kiln_keep.sh stop > /dev/null 2>&1
bash wp84_vanilla_keep.sh stop > /dev/null 2>&1
pkill -f "mc-client/game/"; pkill -f "mc-client/game2/"
sleep 2
bash wp84_kiln_keep.sh > /dev/null 2>&1
bash wp84_vanilla_keep.sh > /dev/null 2>&1
sleep ${RESTART_WAIT:-40}
bash switch.sh K split
