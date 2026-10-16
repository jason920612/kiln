#!/bin/bash
# push the playthrough tools from the Windows worktree to the VM's ~/wp68
cd "$(dirname "$0")" && scp -q -o BatchMode=yes *.py *.sh ClientCtl.java test@192.168.1.169:wp68/ && ssh -n -o BatchMode=yes test@192.168.1.169 'sed -i "s/\r$//" ~/wp68/*.py ~/wp68/*.sh'
