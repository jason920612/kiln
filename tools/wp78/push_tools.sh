#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp78 (wp58, wp72, wp74 tools first, the wp78 ones on top)
T="$(cd "$(dirname "$0")/.." && pwd)"
H=test@192.168.1.169
ssh -n -o BatchMode=yes $H 'mkdir -p ~/wp78'
scp -q -o BatchMode=yes "$T"/wp58/*.py "$T"/wp58/ClientCtl.java $H:wp78/ && scp -q -o BatchMode=yes "$T"/wp72/*.py $H:wp78/ && scp -q -o BatchMode=yes "$T"/wp74/*.py $H:wp78/ && scp -q -o BatchMode=yes "$T"/wp78/*.sh "$T"/wp78/*.py $H:wp78/ && scp -q -o BatchMode=yes -r "$T"/wp78/agent "$T"/wp72/SlimeChunks.java $H:wp78/ && ssh -n -o BatchMode=yes $H 'cd ~/wp78; rm -f *68*.sh *72*.sh *74*.sh; sed -i "s/wp72/wp78/g; s/wp74/wp78/g" p72*.py p74*.py; sed -i "s/\r$//" *.py *.sh agent/*; chmod +x *.sh agent/*.sh'
