#!/bin/bash
cd ~/wp74
R() { name=$1; shift; echo "== $name $(date +%T)" >> rest3_progress.txt; ./run.sh "$@" > rest3_$name.txt 2>&1; echo "-- $name done $(date +%T)" >> rest3_progress.txt; }
: > rest3_progress.txt
R back w15.py
KILLBLOCK=1 R enderman p74_farm.py enderman 30 1000
R back2 w15.py
R hall p74_hall.py 12 2
R village p74_village.py 4 12 2000
echo all done >> rest3_progress.txt
