#!/bin/bash
# wp73: flight benchmark pairs (base, new) with the server's per-thread CPU sampled (thr73.py), so the tick's own CPU
# time per tick can be compared on a busy machine (wall-clock MSPT follows the other programs' load).
#   cpu73.sh BASE_BINDIR NEW_BINDIR CASE PAIRS
base="$1"; new="$2"; case_="$3"; pairs="${4:-2}"
tools="$(cd "$(dirname "$0")" && pwd)"
out=~/wp73/cpu; mkdir -p "$out"
for i in $(seq "$pairs"); do
  for v in base new; do
    bin=$base; [ $v = new ] && bin=$new
    tag="c_${case_}_${v}$i"
    python3 "$tools/thr73.py" kiln 240 > "$out/$tag.thr" 2>&1 &
    "$tools/fly73.sh" "$bin" "$tag" "$case_" > "$out/$tag.log" 2>&1
    wait
  done
done
