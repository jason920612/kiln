#!/bin/bash
cd ~/wp86
export KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORK=/home/test/kiln/work
cat > sp_base.properties <<PROP
server-ip=127.0.0.1
online-mode=false
level-seed=86
white-list=false
max-tick-time=-1
view-distance=4
simulation-distance=4
pause-when-empty-seconds=-1
gamemode=creative
difficulty=peaceful
PROP
(cat sp_base.properties; echo server-port=25595) > sp_van.properties
(cat sp_base.properties; echo server-port=25589) > sp_kil.properties
./vanilla.sh start ~/wp86/sp_van.properties >/dev/null 2>&1
./kiln.sh start ~/wp86/sp_kil.properties >/dev/null 2>&1
for i in $(seq 1 90); do grep -q "Done (" van/server.log 2>/dev/null && grep -q "Done (" kil/server.log 2>/dev/null && break; sleep 1; done
sleep 2
echo "== vanilla"; python3 spawn_probe.py 127.0.0.1 25595 van/console
echo "== kiln";    python3 spawn_probe.py 127.0.0.1 25589 kil/console
./vanilla.sh stop >/dev/null 2>&1; ./kiln.sh stop >/dev/null 2>&1
