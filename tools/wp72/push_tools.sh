#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp72 (wp58 tools first, the wp72 ones (same names) on top)
T="$(cd "$(dirname "$0")/.." && pwd)"
H=test@192.168.1.169
scp -q -o BatchMode=yes "$T"/wp58/*.py "$T"/wp58/ClientCtl.java $H:wp72/ && scp -q -o BatchMode=yes "$T"/wp72/*.sh "$T"/wp72/*.py $H:wp72/ && ssh -n -o BatchMode=yes $H 'cd ~/wp72; rm -f *68*.sh; sed -i "s/\r$//" *.py *.sh; chmod +x *.sh'
