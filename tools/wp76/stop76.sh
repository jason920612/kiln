#!/bin/bash
# stop76.sh: stops this session's bench runs and servers (only processes started from ~/wp76).
for pid in $(pgrep -f "tools/bench76.py run"); do
  [ "$pid" != "$$" ] && grep -q "bench76" /proc/$pid/cmdline 2>/dev/null && ! grep -q "stop76" /proc/$pid/cmdline && kill "$pid"
done
sleep 1
~/wp76/tools/servers76.sh K stop
~/wp76/tools/servers76.sh V stop
pkill -f "wp76/bin/.*/kiln-bot" 2>/dev/null
true
