#!/bin/bash
# setsim.sh N: restart both servers (same worlds) with simulation distance N (view distance stays 8) and reconnect the clients
cd ~/wp84
sed -i "s/^simulation-distance=.*/simulation-distance=$1/" vanilla/server.properties
export SIMDIST=$1
bash wp84_kiln_keep.sh stop > /dev/null 2>&1
bash wp84_vanilla_keep.sh stop > /dev/null 2>&1
pkill -f "mc-client/[g]ame/"; pkill -f "mc-client/[g]ame2/"
sleep 2
bash wp84_kiln_keep.sh > /dev/null 2>&1
bash wp84_vanilla_keep.sh > /dev/null 2>&1
sleep ${RESTART_WAIT:-40}
bash switch.sh K split
