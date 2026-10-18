#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp80 (wp58, wp72, wp74 tools first, the wp80 ones on top)
T="$(cd "$(dirname "$0")/.." && pwd)"
H=test@192.168.1.169
ssh -n -o BatchMode=yes $H 'mkdir -p ~/wp80'
scp -q -o BatchMode=yes "$T"/wp58/*.py "$T"/wp58/ClientCtl.java $H:wp80/ && scp -q -o BatchMode=yes "$T"/wp72/*.py $H:wp80/ && scp -q -o BatchMode=yes "$T"/wp74/*.py $H:wp80/ && scp -q -o BatchMode=yes "$T"/wp78/*.py $H:wp80/ && scp -q -o BatchMode=yes "$T"/wp80/*.sh "$T"/wp80/*.py $H:wp80/ && scp -q -o BatchMode=yes -r "$T"/wp80/agent "$T"/wp72/SlimeChunks.java $H:wp80/ && ssh -n -o BatchMode=yes $H 'cd ~/wp80; rm -f *68*.sh *72*.sh *74*.sh; sed -i "s/wp72/wp80/g; s/wp74/wp80/g; s/wp78/wp80/g" p72*.py p74*.py p78*.py; sed -i "s/wp6[0-9]/wp80/g; s/wp5[0-9]/wp80/g" p6*.py p59.py p60.py x_*.py; sed -i "s/\r$//" *.py *.sh agent/*; chmod +x *.sh agent/*.sh'
