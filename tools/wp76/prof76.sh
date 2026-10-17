#!/bin/bash
# wp76: perf profile of Kiln (all threads) while a bench76 scene holds.
#   prof76.sh BIN NAME SCENE [SECONDS] [DELAY]
# Runs bench76.py run K -k SCENE --window SECONDS+40 with BIN; once the scene's function ran, waits DELAY
# seconds (default 15) and records SECONDS (default 30). Report in ~/wp76/prof/NAME.txt: self time, then
# inclusive time of the functions matching $FOCUS.
bin="$1"; name="$2"; scene="$3"; secs="${4:-30}"; delay="${5:-15}"
tools="$(cd "$(dirname "$0")" && pwd)"
mkdir -p ~/wp76/prof
python3 "$tools/bench76.py" run K -k "$scene" --window $((secs + delay + 10)) --bin "$bin" > ~/wp76/prof/$name.bench.log 2>&1 &
benchpid=$!
fn="${scene%%[0-9]*}"
for _ in $(seq 600); do grep -q "Running function wp76:$scene\b\|Running function wp76:${fn}" ~/wp76/K/server.log 2>/dev/null && break; sleep 1; done
sleep "$delay"
pid=$(pgrep -x kiln-run76 | head -1)
sudo -n perf record -F ${FREQ:-499} --call-graph ${CG:-dwarf,16384} -p "$pid" -o ~/wp76/prof/$name.data -- sleep "$secs" > /dev/null 2>&1
sudo -n chown test ~/wp76/prof/$name.data
wait $benchpid
perf report -i ~/wp76/prof/$name.data --no-children --percent-limit 0.4 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -80 > ~/wp76/prof/$name.txt
echo "--- inclusive" >> ~/wp76/prof/$name.txt
perf report -i ~/wp76/prof/$name.data --children --percent-limit 0.5 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | grep -E "${FOCUS:-kiln}" | head -90 >> ~/wp76/prof/$name.txt
grep "^\[K\]" ~/wp76/prof/$name.bench.log >> ~/wp76/prof/$name.txt
echo done >> ~/wp76/prof/$name.txt
