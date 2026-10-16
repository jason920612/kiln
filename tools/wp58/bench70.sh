#!/bin/bash
# on the VM: bench70.sh <out-name> <bench70.py run args...>: runs detached, output in ~/wp70/<out-name>.txt (poll it)
cd "$(dirname "$0")"
out="$1"; shift
setsid nohup python3 bench70.py run "$@" > ~/wp70/"$out".txt 2>&1 < /dev/null &
echo "running, output ~/wp70/$out.txt"
