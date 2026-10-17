#!/bin/bash
# on the VM: the lock-step scenarios one after the other (both servers + the two clients are up, nobody else drives them); outputs ~/wp74/rest_*.txt
cd ~/wp74
R() { name=$1; shift; echo "== $name $(date +%T)" >> rest_progress.txt; ./run.sh "$@" > rest_$name.txt 2>&1; echo "-- $name done $(date +%T)" >> rest_progress.txt; }
: > rest_progress.txt
R endermite p74_misc.py endermite 36
R turtles p74_misc.py turtles 18
KILLBLOCK=1 R gold p74_farm.py gold 30 1000
KILLBLOCK=1 R enderman p74_farm.py enderman 30 1000
R village p74_village.py 4 12 2000
R hall p74_hall.py 12 2
echo all done >> rest_progress.txt
