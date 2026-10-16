#!/bin/bash
# on the VM: prof70.sh <kiln binary> <name> <command> [seconds]: starts the wp70 Kiln server (an idle bot in spectator above
# the test area), then records `perf` (dwarf call graphs) while the console runs <command>. Report in ~/wp70/<name>.txt.
bin="$1"; name="$2"; cmd="$3"; secs="${4:-15}"
tools="$(cd "$(dirname "$0")" && pwd)"
"$tools/wp70_servers.sh" K start "$bin"
c=~/wp70/K/console
sleep 8
for l in "difficulty peaceful" "gamerule spawn_mobs false" "gamerule advance_time false" "gamerule advance_weather false" \
         "forceload add 0 0 111 95" "forceload add 192 192 239 239"; do echo "$l" > $c; done
setsid nohup ~/wt/wp70-command-perf/target/release/kiln-bot --addr 127.0.0.1:25590 --count 1 --behavior idle --duration 600 \
  --name-prefix Bench --report-interval 3600 > ~/wp70/bot_prof.log 2>&1 < /dev/null &
sleep 6
echo "gamemode spectator Bench0" > $c; echo "tp Bench0 16 200 16" > $c
sleep 20
pid=$(pgrep -x kiln-run70)
sudo -n perf record -F 499 --call-graph dwarf,32768 -p "$pid" -o ~/wp70/$name.data -- sleep "$secs" > /dev/null 2>&1 &
sleep 1
echo "$cmd" > $c
wait %1
"$tools/wp70_servers.sh" K stop
pkill -f "name-prefix Bench --report-interval 3600" 2>/dev/null
sudo -n chown test ~/wp70/$name.data
perf report -i ~/wp70/$name.data --no-children --percent-limit 0.5 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -60 > ~/wp70/$name.txt
perf report -i ~/wp70/$name.data --children --percent-limit 2 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -80 >> ~/wp70/$name.txt
grep -n "slow tick\|mspt" ~/wp70/K/server.log | tail -3 >> ~/wp70/$name.txt
echo done >> ~/wp70/$name.txt
