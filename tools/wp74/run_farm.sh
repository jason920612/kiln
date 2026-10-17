#!/bin/bash
# run_farm.sh OUT script.py args...: copy + run in the foreground on the VM (env vars are passed through ssh as FARM_ENV)
out="$1"; f="$2"; shift 2; b=$(basename "$f")
scp -q -o BatchMode=yes "$f" test@192.168.1.169:wp74/$b
ssh -n -o BatchMode=yes test@192.168.1.169 "sed -i 's/\r\$//' ~/wp74/$b; cd ~/wp74; $FARM_ENV ./run.sh $b $* > $out 2>&1; tail -n 40 $out | cut -c1-300"
