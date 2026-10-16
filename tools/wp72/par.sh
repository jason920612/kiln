#!/bin/bash
# par.sh script.py [args]: run a one-actor script against Kiln (agent 25999) and vanilla (agent 26000) at the same time (switch.sh K split)
cd "$(dirname "$0")"
f="$1"; b=$(basename "$f"); shift
scp -q -o BatchMode=yes "$f" test@192.168.1.169:wp72/$b
ssh -n -o BatchMode=yes test@192.168.1.169 "sed -i 's/\r\$//' ~/wp72/$b; cd ~/wp72; SPLIT=1 TARGET=K KILN_AGENT_PORT=25999 ./run.sh $b $* > out_K.txt 2>&1 & SPLIT=1 TARGET=V KILN_AGENT_PORT=26000 ./run.sh $b $* > out_V.txt 2>&1; wait; echo ==K; tail -n \${TAILN:-30} out_K.txt | cut -c1-\${W:-400}; echo ==V; tail -n \${TAILN:-30} out_V.txt | cut -c1-\${W:-400}"
