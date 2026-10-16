#!/bin/bash
# on the VM: switch.sh K|V [both]  (re)start the actor client (and with `both` the observer) against Kiln (K) or vanilla (V)
cd ~/wp72
# usage: switch.sh K|V [both|split]   split: the actor (agent 25999) on Kiln and a second client also named KilnTest (agent 26000) on vanilla
[ "$1" = V ] && export KILN_CLIENT_PORT=25595 || export KILN_CLIENT_PORT=25589
if [ "$2" = split ]; then
  KILN_CLIENT_PORT=25589 bash vm_client68.sh > /dev/null 2>&1 < /dev/null; sleep 20
  OBS_NAME=KilnTest KILN_CLIENT_PORT=25595 bash vm_observer68.sh > /dev/null 2>&1 < /dev/null
  set -- "$1" both
else
  bash vm_client68.sh > /dev/null 2>&1 < /dev/null
  if [ "$2" = both ]; then sleep 20; bash vm_observer68.sh > /dev/null 2>&1 < /dev/null; fi
fi
for i in $(seq 1 60); do
  sleep 2
  if KILN_PLAY_DIR=wp72 PYTHONPATH=. python3 -c "
from ctl import C
c=C(25999); s=c.state(0); import sys; sys.exit(0 if s.get('inGame') else 1)" 2>/dev/null; then echo actor ready; break; fi
done
if [ "$2" = both ]; then
for i in $(seq 1 60); do
  sleep 2
  if KILN_PLAY_DIR=wp72 PYTHONPATH=. python3 -c "
from ctl import C
c=C(26000); s=c.state(0); import sys; sys.exit(0 if s.get('inGame') else 1)" 2>/dev/null; then echo observer ready; break; fi
done
fi
