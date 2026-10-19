#!/bin/bash
# On the VM: starts vanilla (25595) and Kiln (25589) with the same server.properties plus the lines given,
# optionally with players online (PLAYERS="Alice Bob") and a server-icon.png (ICON=path), then prints the
# server list JSON both give (and the legacy ping reply), so they can be compared.
#   usage: status_cmp.sh <label> [property lines...]
# Env: PLAYERS, ICON, KILN_BIN.
label="$1"; shift
cd ~/wp86
base=$(mktemp)
cat > $base <<EOF
server-ip=127.0.0.1
online-mode=false
level-seed=86
white-list=false
max-tick-time=-1
view-distance=4
simulation-distance=4
motd=Hello from wp86 $label
pause-when-empty-seconds=-1
EOF
for l in "$@"; do echo "$l" >> $base; done
export KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORK=/home/test/kiln/work
sed 's/$//' $base > van_st.properties; echo "server-port=25595" >> van_st.properties
sed 's/$//' $base > kil_st.properties; echo "server-port=25589" >> kil_st.properties
./vanilla.sh start ~/wp86/van_st.properties > /dev/null 2>&1
./kiln.sh start ~/wp86/kil_st.properties > /dev/null 2>&1
for i in $(seq 1 90); do grep -q "Done (" van/server.log 2>/dev/null && grep -q "listening on" kil/server.log 2>/dev/null && break; sleep 1; done
sleep 2
pids=""
if [ -n "$PLAYERS" ]; then
  for p in $PLAYERS; do
    python3 - "$p" <<'PY' &
import sys, time
sys.path.insert(0, ".")
import mcclient as m
name = sys.argv[1]
cs = []
for port in (25595, 25589):
    c = m.Client("127.0.0.1", port, name).login()
    c.until_play()
    cs.append(c)
end = time.time() + 14
while time.time() < end:
    for c in cs:
        c.drain(0.3)
PY
    pids="$pids $!"
  done
  sleep 7
fi
echo "== $label vanilla"; python3 slp.py 127.0.0.1 25595 | sed -n 2p | cut -c1-400
echo "== $label kiln";    python3 slp.py 127.0.0.1 25589 | sed -n 2p | cut -c1-400
echo "== $label vanilla legacy"; python3 slp.py 127.0.0.1 25595 --legacy | tail -1
echo "== $label kiln legacy";    python3 slp.py 127.0.0.1 25589 --legacy | tail -1
echo "-- vanilla log"; grep -a "icon\|ERROR\|WARN" van/server.log | cut -c1-200 | head -5
echo "-- kiln log"; grep -a "icon\|ERROR\|WARN" kil/server.log | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-200 | head -5
for p in $pids; do wait $p 2>/dev/null; done
./vanilla.sh stop > /dev/null 2>&1; ./kiln.sh stop > /dev/null 2>&1
rm -f $base
