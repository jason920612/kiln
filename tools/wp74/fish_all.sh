#!/bin/bash
# on the VM: the fishing variants one after the other, Kiln (agent 25999) and vanilla (26000) at the same time; results in ~/wp74/fish_results.txt
cd ~/wp74
N=${1:-120}
[ -n "$KEEP" ] || : > fish_results.txt
for v in "closed none" "open none"; do
  set -- $v
  SPLIT=1 TARGET=K KILN_AGENT_PORT=25999 ./run.sh p74_fish.py $N 100 $1 $2 > out_fK.txt 2>&1 &
  SPLIT=1 TARGET=V KILN_AGENT_PORT=26000 ./run.sh p74_fish.py $N 100 $1 $2 > out_fV.txt 2>&1
  wait
  grep -h RESULT out_fK.txt out_fV.txt >> fish_results.txt
done
echo done >> fish_results.txt
