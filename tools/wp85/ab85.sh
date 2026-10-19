#!/bin/bash
# wp85 (on the VM): the tick cost of main against this branch, alternating runs of the same synthetic load (50 walking players in 10 groups on noise terrain).
#   tools/wp85/ab85.sh [rounds]    base = ~/wt/main, new = ~/wt/wp85-worldgen-packs (build the sim_load example in both first)
export KILN_DATAPACK=$HOME/kiln/work/generated KILN_WORK=$HOME/kiln/work
for r in $(seq 1 ${1:-6}); do
  for w in base new; do
    case $w in base) d=$HOME/wt/main;; new) d=$HOME/wt/wp85-worldgen-packs;; esac
    cd $d
    out=$(./target/release/examples/sim_load --players 50 --groups 10 --spacing 200 --radius 24 --ticks 600 --view-distance 6 --behavior walk --noise 85 2>&1)
    echo "$w r$r: $(echo "$out" | grep 'measured ticks' | sed 's/.*mspt/mspt/') | $(echo "$out" | grep -o 'state hash [0-9a-f]*') | load $(cut -d' ' -f1 /proc/loadavg)"
  done
done
