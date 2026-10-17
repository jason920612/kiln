#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp74 (wp58 tools first, the wp72 and wp74 ones on top)
T="$(cd "$(dirname "$0")/.." && pwd)"
H=test@192.168.1.169
ssh -n -o BatchMode=yes $H 'mkdir -p ~/wp74'
scp -q -o BatchMode=yes "$T"/wp58/*.py "$T"/wp58/ClientCtl.java $H:wp74/ && scp -q -o BatchMode=yes "$T"/wp72/*.py $H:wp74/ && scp -q -o BatchMode=yes "$T"/wp74/*.sh "$T"/wp74/*.py $H:wp74/ && scp -q -o BatchMode=yes -r "$T"/wp74/agent "$T"/wp72/SlimeChunks.java $H:wp74/ && ssh -n -o BatchMode=yes $H 'cd ~/wp74; rm -f *68*.sh *72*.sh; sed -i "s/wp72/wp74/g" p72*.py; sed -i "s/\r$//" *.py *.sh agent/*; chmod +x *.sh agent/*.sh'
