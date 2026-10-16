#!/bin/bash
# on the VM: restart the Kiln server from the wp68 checkout's release build and reconnect only the actor client (agent 25999)
cd ~/wp68
export PATH=$HOME/.cargo/bin:$PATH
(cd ~/wt/wp68-playthrough6 && CARGO_BUILD_JOBS=5 KILN_WORK=$HOME/kiln/work cargo build --release -p kiln-server 2>&1 | tail -1)
(setsid bash wp68_start.sh > kstart.out 2>&1 < /dev/null &)
sleep 12
KILN_CLIENT_PORT=25589 bash vm_client68.sh > /dev/null 2>&1 < /dev/null
for i in $(seq 1 60); do
  sleep 2
  if KILN_PLAY_DIR=wp68 PYTHONPATH=. python3 -c "
from ctl import C
c=C(25999); s=c.state(0); import sys; sys.exit(0 if s.get('inGame') else 1)" 2>/dev/null; then echo actor ready; break; fi
done
