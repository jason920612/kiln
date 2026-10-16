#!/bin/bash
# wp69: run_k.sh TAG REPS: REPS fresh Kiln servers, each runs the farm for 6 days in 4 pens; results in /tmp/w69_K_TAG_<rep>.json
cd ~/wp69
tag=$1; reps=$2
for rep in $(seq 1 $reps); do
  bash ~/wt/wp69-iron-farm/tools/wp69/k69_start.sh stop > /dev/null 2>&1 < /dev/null
  KILN_SEED=$((100 + rep + ${SEEDBASE:-0})) bash ~/wt/wp69-iron-farm/tools/wp69/k69_start.sh > /dev/null 2>&1 < /dev/null
  for i in $(seq 1 60); do grep -q "listening\|Done\|ready" server.log 2>/dev/null && break; sleep 2; done
  sleep 5
  (exec 3<>console; echo reload >&3)
  sleep 3
  TARGET=K timeout 3000 python3 rt_farm69.py run 6 4 /tmp/w69_K_${tag}_$rep.json 2>&1 | tail -1
done
echo ALLDONE
