#!/bin/bash
# q.sh K|V script.py  : copy a local script to the VM's ~/wp68 and run it against the target
f="$2"; b=$(basename "$f")
scp -q -o BatchMode=yes "$f" test@192.168.1.169:wp68/$b && ssh -n -o BatchMode=yes test@192.168.1.169 "sed -i 's/\r\$//' ~/wp68/$b; cd ~/wp68; TARGET=$1 ./run.sh $b" 2>&1
