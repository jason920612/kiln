#!/bin/bash
# wp73: perf profile of the tick thread (`sim`) during a flight benchmark on the VM.
#   prof73.sh BINDIR NAME CASE [SECONDS] [DELAY]
# Runs fly73.sh BINDIR NAME CASE, records `perf` on the server's tick thread for SECONDS (default 60) starting
# DELAY seconds (default 70) after the server is up; report in ~/wp73/prof/NAME.txt (self time, then callers of the
# functions given in $FOCUS, a regex).
bin="$1"; name="$2"; case_="$3"; secs="${4:-60}"; delay="${5:-70}"
tools="$(cd "$(dirname "$0")" && pwd)"
mkdir -p ~/wp73/prof
"$tools/fly73.sh" "$bin" "$name" "$case_" > ~/wp73/prof/$name.bench.log 2>&1 &
benchpid=$!
for _ in $(seq 120); do pid=$(pgrep -f "^$bin/kiln( |$)" | head -1); [ -n "$pid" ] && break; sleep 1; done
sleep "$delay"
tid=$(ps -L -o tid=,comm= -p "$pid" | awk '$2 == "sim" {print $1}' | head -1)
sudo -n perf record -F ${FREQ:-999} --call-graph ${CG:-dwarf,32768} -t "$tid" -o ~/wp73/prof/$name.data -- sleep "$secs" > /dev/null 2>&1
sudo -n chown test ~/wp73/prof/$name.data
wait $benchpid
perf report -i ~/wp73/prof/$name.data --no-children --percent-limit 0.5 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | head -70 > ~/wp73/prof/$name.txt
perf report -i ~/wp73/prof/$name.data --children --percent-limit 0.3 --stdio -g none 2>/dev/null | grep -v "^#" | grep -v "^$" | grep -E "${FOCUS:-unload|store_entities|Poi|poi|compress|write_named|to_nbt|save_in|chunk_unloaded|stash|encode}" | head -60 >> ~/wp73/prof/$name.txt
grep "^mspt\|^slow" ~/wp73/prof/$name.bench.log >> ~/wp73/prof/$name.txt
echo done >> ~/wp73/prof/$name.txt
