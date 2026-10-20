#!/bin/bash
# Starts vanilla (25595, rcon 25660, query 25661) and Kiln (25589, 25662, 25663) with the same properties; runs the probes; prints diffs.
cd ~/wp86
export KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORK=/home/test/kiln/work
sed 's/@PORT@/25595/;s/@RCON@/25660/;s/@QUERY@/25661/' cmp.properties.tmpl > van_cmp.properties
sed 's/@PORT@/25589/;s/@RCON@/25662/;s/@QUERY@/25663/' cmp.properties.tmpl > kil_cmp.properties
./vanilla.sh start ~/wp86/van_cmp.properties >/dev/null 2>&1
./kiln.sh start ~/wp86/kil_cmp.properties >/dev/null 2>&1
for i in $(seq 1 90); do grep -q "RCON running" van/server.log 2>/dev/null && grep -q "Done (" kil/server.log 2>/dev/null && break; sleep 1; done
sleep 3
norm() { sed -e 's/token [0-9]*/token T/' -e 's/hostport.x00255[0-9]*/hostport PORT/' -e 's/xfbc/PORTc/;s/xf5c/PORTc/;s/xf3c/PORTc/' -e 's/PORTc/PORTc/' "$1" | cut -c1-400; }
python3 chat_probe.py 127.0.0.1 25595 --op-fifo van/console > van_chat.txt 2>&1
python3 chat_probe.py 127.0.0.1 25589 --op-fifo kil/console > kil_chat.txt 2>&1
echo "== chat diff:"; diff van_chat.txt kil_chat.txt && echo identical
python3 chat_errors.py 127.0.0.1 25595 > van_err.txt 2>&1
python3 chat_errors.py 127.0.0.1 25589 > kil_err.txt 2>&1
echo "== chat errors diff:"; diff van_err.txt kil_err.txt && echo identical
python3 rcon_probe.py 127.0.0.1 25660 test86 25661 > van_rcon.txt 2>&1
python3 rcon_probe.py 127.0.0.1 25662 test86 25663 > kil_rcon.txt 2>&1
echo "== rcon/query diff (ports, tokens and the help listing normalized):"; diff <(norm van_rcon.txt | grep -v "^fill huge") <(norm kil_rcon.txt | grep -v "^fill huge") && echo identical
./vanilla.sh stop >/dev/null 2>&1; ./kiln.sh stop >/dev/null 2>&1
