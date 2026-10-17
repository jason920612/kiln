#!/bin/bash
# on the VM: village + hall again after getting both players back alive into the overworld
cd ~/wp74
R() { name=$1; shift; echo "== $name $(date +%T)" >> rest2_progress.txt; ./run.sh "$@" > rest2_$name.txt 2>&1; echo "-- $name done $(date +%T)" >> rest2_progress.txt; }
: > rest2_progress.txt
R hall p74_hall.py 12 2
R village p74_village.py 4 12 2000
echo all done >> rest2_progress.txt
