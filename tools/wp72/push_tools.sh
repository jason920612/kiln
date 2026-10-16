#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp72
cd "$(dirname "$0")/../wp58" && scp -q -o BatchMode=yes *.py *.sh ClientCtl.java ../wp72/*.sh ../wp72/*.py test@192.168.1.169:wp72/ && ssh -n -o BatchMode=yes test@192.168.1.169 'sed -i "s/\r$//" ~/wp72/*.py ~/wp72/*.sh'
