#!/bin/bash
# wp71: run_k71.sh TAG REPS: REPS fresh Kiln servers, each runs the farm (DAYS days, PENS pens) from rt_farm71.py; results in ~/w71/K_TAG_<rep>.json
# (env ALIGN=1 PHASE=n, NOZOMBIE=1, DEEP=n, CLEARBELOW=n pass through)
cd ~/wt/wp71-farm-rng/tools/wp71
tag=$1; reps=$2
for rep in $(seq 1 $reps); do
  bash k71_start.sh stop > /dev/null 2>&1 < /dev/null
  KILN_SEED=$((100 + rep + ${SEEDBASE:-0})) bash k71_start.sh > /dev/null 2>&1 < /dev/null
  for i in $(seq 1 60); do grep -q "listening\|Done\|ready" ~/wp71/server.log 2>/dev/null && break; sleep 2; done
  sleep 5
  (exec 3<>~/wp71/console; echo reload >&3)
  sleep 3
  TARGET=K timeout 6000 python3 rt_farm71.py run ${DAYS:-6} ${PENS:-24} ~/w71/K_${tag}_$rep.json 2>&1 | tail -1
  bash k71_start.sh stop > /dev/null 2>&1 < /dev/null
done
echo ALLDONE
