#!/bin/bash
# q.sh K|V script.py [args]: copy a local script to the VM's ~/wp84 and run it against the target (or both for lock scripts)
f="$2"; b=$(basename "$f"); t="$1"; shift 2
scp -q -o BatchMode=yes "$f" test@192.168.1.169:wp84/$b && ssh -n -o BatchMode=yes test@192.168.1.169 "sed -i 's/\r\$//' ~/wp84/$b; cd ~/wp84; TARGET=$t ./run.sh $b $*" 2>&1
