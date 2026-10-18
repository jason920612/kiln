#!/bin/bash
# wp76 final numbers (quiet VM): every item scene on Kiln before (BASE) and after (NEW) and on vanilla 26.3, then
# survival_bench (50 players) and one overworld elytra flyer before and after. Results in ~/wp76/fb/.
BASE=~/wp76/bin/base NEW=~/wp76/bin/final
F=~/wp76/fb; mkdir -p $F; rm -f $F/done
cd ~/wp76
S=scatter1000,scatter5000,scatter10000,hut1000,hut5000,hut10000,pile1000,pile5000,pile10000,farm5000,merge5000,tnt,stream,hopper5000,reload5000,lavafall,hazard
uptime > $F/load_start.txt
python3 tools/bench76.py run K -k $S --bin $NEW/kiln --out $F/K_new.json > $F/K_new.log 2>&1
python3 tools/bench76.py run V -k $S --out $F/V.json > $F/V.log 2>&1
python3 tools/bench76.py run K -k $S --bin $BASE/kiln --out $F/K_base.json > $F/K_base.log 2>&1
for t in base final; do
  ~/wp76/tools/fly76.sh ~/wp76/bin/$t surv50_$t surv50 > $F/surv50_$t.log 2>&1
  ~/wp76/tools/fly76.sh ~/wp76/bin/$t ow1_$t ow1 > $F/ow1_$t.log 2>&1
done
uptime > $F/load_end.txt
echo ALLDONE > $F/done
