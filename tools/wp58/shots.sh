#!/bin/bash
# on the VM: shots.sh K|V name...  -> ~/wp68/shots/<name>_<K|V>.jpg  (scene by p68_shots.py, screenshot of the Xvfb display)
cd ~/wp68; mkdir -p shots
T=$1; shift
for n in "$@"; do
  SPLIT=1 TARGET=$T KILN_AGENT_PORT=25999 ./run.sh p68_shots.py $n > /dev/null 2>&1
  DISPLAY=:99 import -window root shots/${n}_$T.png
  convert shots/${n}_$T.png -quality 72 shots/${n}_$T.jpg && rm shots/${n}_$T.png
done
ls -la shots
