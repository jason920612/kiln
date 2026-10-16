#!/bin/bash
# q.sh K|V script.py  : copy a local script to the VM's ~/wp72 and run it against the target
f="$2"; b=$(basename "$f")
scp -q -o BatchMode=yes "$f" test@192.172.1.169:wp72/$b && ssh -n -o BatchMode=yes test@192.172.1.169 "sed -i 's/\r\$//' ~/wp72/$b; cd ~/wp72; TARGET=$1 ./run.sh $b" 2>&1
