#!/bin/bash
# wp73 final measurements on a quiet VM, base and new interleaved: forceload (Kiln base, Kiln new, vanilla) x2,
# 1 flyer overworld and nether x2, 8 flyers, survival 50, 8000 chests x2. Results in ~/wp73/final/.
#   final73.sh BASE_BINDIR NEW_BINDIR
base="$1"; new="$2"
out=~/wp73/final; mkdir -p "$out"
tools="$(cd "$(dirname "$0")" && pwd)"
log() { echo "$(date +%T) $* (load $(cut -d' ' -f1 /proc/loadavg))" >> "$out/progress"; }
cd "$tools" || exit 1
for i in 1 2; do
  log "forceload $i"
  python3 fl73.py K --bin "$base/kiln" --out "$out/fl_base$i.json" > "$out/fl_base$i.log" 2>&1
  python3 fl73.py K --bin "$new/kiln" --out "$out/fl_new$i.json" > "$out/fl_new$i.log" 2>&1
  python3 fl73.py V --out "$out/fl_v$i.json" > "$out/fl_v$i.log" 2>&1
done
for c in ow1 nether1; do
  for i in 1 2; do
    log "$c $i"
    ./fly73.sh "$base" "f_${c}_base$i" "$c" > "$out/${c}_base$i.log" 2>&1
    ./fly73.sh "$new" "f_${c}_new$i" "$c" > "$out/${c}_new$i.log" 2>&1
  done
done
log ow8
./fly73.sh "$base" f_ow8_base ow8 > "$out/ow8_base.log" 2>&1
./fly73.sh "$new" f_ow8_new ow8 > "$out/ow8_new.log" 2>&1
log surv50
./fly73.sh "$base" f_s50_base surv50 > "$out/s50_base.log" 2>&1
./fly73.sh "$new" f_s50_new surv50 > "$out/s50_new.log" 2>&1
for i in 1 2; do
  log "chests $i"
  python3 chest73.py --bin "$base/kiln" >> "$out/chest_base.log" 2>&1
  python3 chest73.py --bin "$new/kiln" >> "$out/chest_new.log" 2>&1
done
log done
