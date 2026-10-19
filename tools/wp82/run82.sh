#!/bin/bash
# wp82 (on the VM): run82.sh SCENARIO ROUNDS TICKS BOX [SIMDIST]  fresh Kiln + vanilla (seed 77) with the spawn counters, one bot each, the farm scenario in lock step,
# then the counters side by side. Output in ~/wp82/run_<scenario>.out
sc=$1; rounds=${2:-6}; ticks=${3:-1200}; box=$4; sim=${5:-6}
cd ~/wp82
export BOX=$box COUNTERS=1 AGENT=1 W72=1
./bot.sh stop > /dev/null 2>&1
./sv.sh K fresh $sim > /dev/null 2>&1
./sv.sh V fresh $sim > /dev/null 2>&1
sleep 45
./bot.sh K W 1 > /dev/null 2>&1
./bot.sh V W 1 > /dev/null 2>&1
sleep 15
python3 farm82.py $sc $rounds $ticks
./bot.sh stop > /dev/null 2>&1
./sv.sh V stop > /dev/null 2>&1
sleep 3
./sv.sh K stop > /dev/null 2>&1
echo "== validPosition IN-BOX"; python3 cmp82.py "validPosition IN-BOX"
echo "== global/categories"; python3 cmp82.py global categories
echo "== pick"; python3 cmp82.py pick
echo "== validMob"; python3 cmp82.py validMob
