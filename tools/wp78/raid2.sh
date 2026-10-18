#!/bin/bash
# on the VM: raid2.sh K|V — two real players (KilnTest 25999, Obs 26000) run the wp66 raid script at once against one server (village of seed 78 at 320,-400)
cd ~/wp78
export TARGET=$1 KILN_PLAY_DIR=wp78 PYTHONPATH=. VX=${VX:-320} VZ=${VZ:--400} DIFFICULTY=${DIFFICULTY:-normal} OMEN=${OMEN:-2}
ACTOR=KilnTest AGENT_PORT=25999 nohup python3 -u p66_raid.py /tmp/raidA_$1.json > out_raidA_$1.txt 2>&1 < /dev/null &
ACTOR=Obs AGENT_PORT=26000 SMALL=1 nohup python3 -u p66_raid.py /tmp/raidB_$1.json > out_raidB_$1.txt 2>&1 < /dev/null &
echo started
