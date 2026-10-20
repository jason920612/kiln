#!/bin/bash
# wp86 management server probes, from the Windows side: [FRESH=1] run.sh van|kil SCRIPT [extra probe args] > transcript
# Copies the probe tools and the script to the VM (~/wp86m) and runs it against the server there
# (vanilla on 25666 or Kiln on 25669; the test secret is not a real one). With FRESH=1 the server is
# restarted from its props file (van_m.properties / kil_m.properties in ~/wp86m) first, so every
# transcript starts from the same world.
set -e
which=$1; script=$2; shift 2
here=$(cd "$(dirname "$0")" && pwd)
scp -q -o BatchMode=yes "$here/probe.py" "$here/wsrpc.py" "$here/../mcclient.py" test@192.168.1.169:/home/test/wp86m/
scp -q -o BatchMode=yes -r "$here/scripts" test@192.168.1.169:/home/test/wp86m/
case $which in van) port=25666; game=25667; ready="Done (";; kil) port=25669; game=25668; ready="Done";; esac
if [ -n "$FRESH" ]; then
  case $which in
    van) start="./vanilla.sh start /home/test/wp86m/van_m.properties";;
    kil) start="KILN_DATAPACK=/home/test/kiln/work/generated KILN_WORK=/home/test/kiln/work ./kiln.sh start /home/test/wp86m/kil_m.properties";;
  esac
  ssh -o BatchMode=yes test@192.168.1.169 "cd ~/wp86m && $start >/dev/null && for i in \$(seq 1 120); do grep -q '$ready' $which/server.log 2>/dev/null && break; sleep 1; done; sleep 1"
fi
ssh -o BatchMode=yes test@192.168.1.169 "cd ~/wp86m && python3 probe.py 127.0.0.1 $port abcdefghijABCDEFGHIJ0123456789klmnopqrst scripts/$script --console $which/console --game-port $game $*"
