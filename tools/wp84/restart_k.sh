#!/bin/bash
# restart_k.sh: stop Kiln, install the freshly built binary, start it on the same world and reconnect the actor client
cd ~/wp84
bash wp84_kiln_keep.sh stop > /dev/null 2>&1 < /dev/null
cp ~/wt/wp84-playthrough11/target/release/kiln ~/wp84/kiln-run84
bash wp84_kiln_keep.sh > /dev/null 2>&1 < /dev/null
sleep 12
bash actor.sh K 2>&1 < /dev/null | tail -2
