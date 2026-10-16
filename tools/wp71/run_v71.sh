#!/bin/bash
# wp71: run_v71.sh TAG REPS: REPS fresh vanilla servers (with the agent), each runs the farm (DAYS days, PENS pens) from tools/wp71/rt_farm71.py; results in ~/w71/V_TAG_<rep>.json and the agent output in ~/w71/V_TAG_<rep>.agent
# (env ALIGN=1 PHASE=n, NOZOMBIE=1 pass through)
cd ~/wt/wp71-farm-rng/tools/wp71
tag=$1; reps=$2
for rep in $(seq 1 $reps); do
  bash v71_start.sh stop > /dev/null 2>&1 < /dev/null
  W71_OUT=$HOME/w71/V_${tag}_$rep.agent bash v71_start.sh > /dev/null 2>&1 < /dev/null
  for i in $(seq 1 90); do grep -q "Done (" ~/wp71/vanilla/server.log 2>/dev/null && break; sleep 2; done
  sleep 2
  TARGET=V timeout 6000 python3 rt_farm71.py run ${DAYS:-6} ${PENS:-24} ~/w71/V_${tag}_$rep.json 2>&1 | tail -1
  bash v71_start.sh stop > /dev/null 2>&1 < /dev/null
done
echo ALLDONE
