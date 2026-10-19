#!/bin/bash
# on the VM: actor.sh K|V — restart only the actor client (agent 25999, window 960x540) against Kiln (K) or vanilla (V) and wait until it is in the game
cd ~/wp84
pkill -f "mc-client/[g]ame/" ; sleep 2
[ "$1" = V ] && export KILN_CLIENT_PORT=25595 || export KILN_CLIENT_PORT=25589
bash vm_client84.sh > /dev/null 2>&1 < /dev/null
for i in $(seq 1 60); do
  sleep 3
  if KILN_PLAY_DIR=wp84 PYTHONPATH=. python3 -c "
from ctl import C
c=C(25999); s=c.state(0); import sys; sys.exit(0 if s.get('inGame') else 1)" 2>/dev/null; then echo actor ready; exit 0; fi
done
echo actor NOT ready
