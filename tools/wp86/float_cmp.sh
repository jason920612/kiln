#!/bin/bash
# usage: float_cmp.sh <allow-flight true|false> [y offset] [seconds]
cd ~/wp86
export KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORK=/home/test/kiln/work
cat > fl_base.properties <<PROP
server-ip=127.0.0.1
online-mode=false
level-seed=86
white-list=false
max-tick-time=-1
view-distance=4
simulation-distance=4
motd=floating
pause-when-empty-seconds=-1
gamemode=survival
difficulty=peaceful
allow-flight=$1
PROP
(cat fl_base.properties; echo server-port=25595) > fl_van.properties
(cat fl_base.properties; echo server-port=25589) > fl_kil.properties
./vanilla.sh start ~/wp86/fl_van.properties >/dev/null 2>&1
./kiln.sh start ~/wp86/fl_kil.properties >/dev/null 2>&1
for i in $(seq 1 90); do grep -q "Done (" van/server.log 2>/dev/null && grep -q "listening on" kil/server.log 2>/dev/null && break; sleep 1; done
sleep 2
echo "== vanilla allow-flight=$1"; python3 float_probe.py 127.0.0.1 25595 ${2:-10} ${3:-8}
echo "== kiln allow-flight=$1";    python3 float_probe.py 127.0.0.1 25589 ${2:-10} ${3:-8}
grep -a "floating" van/server.log kil/server.log | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-160
./vanilla.sh stop >/dev/null 2>&1; ./kiln.sh stop >/dev/null 2>&1
