#!/bin/bash
# wp69: run_v.sh TAG REPS [ENV=...]: REPS fresh vanilla servers, each runs the farm for 6 days in 4 pens; results in /tmp/w69_V_TAG_<rep>.json
# (env ALIGN=1 etc. pass through to rt_farm69.py).
cd ~/wp69
tag=$1; reps=$2
for rep in $(seq 1 $reps); do
  bash ~/wt/wp69-iron-farm/tools/wp69/v69_start.sh stop > /dev/null 2>&1 < /dev/null
  bash ~/wt/wp69-iron-farm/tools/wp69/v69_start.sh > /dev/null 2>&1 < /dev/null
  for i in $(seq 1 60); do grep -q "Done (" vanilla/server.log 2>/dev/null && break; sleep 2; done
  sleep 2
  TARGET=V timeout 3000 python3 rt_farm69.py run 6 ${PENS:-4} /tmp/w69_V_${tag}_$rep.json 2>&1 | tail -1
done
echo ALLDONE
