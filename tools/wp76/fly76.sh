#!/bin/bash
# wp76: wp73 flight benchmark (wp67's survival_bench flyers) on the VM, port 25590:
#   fly76.sh BINDIR TAG ow1|nether1|ow8|surv50 [extra survival_bench args]
# BINDIR holds kiln, kiln-bot and survival_bench; results in ~/wp76/out/TAG-*.
bin="$1"; tag="$2"; case_="$3"; shift 3
out="$HOME/wp76/out"; mkdir -p "$out"
world="/tmp/wp76-world-$tag"; rm -rf "$world"
common=(--server "$bin/kiln" --bot "$bin/kiln-bot" --datapack /home/test/kiln/work/generated --world "$world" --phase gen
        --port 25590 --slow-print ${SLOW:-10} --out "$out" --tag "$tag")
case "$case_" in
  ow1) args=(--count 0 --flyers 1 --fly-dim minecraft:overworld --fly-center 9000,0 --fly-heading 1,0 --fly-altitude 150 --warmup 20 --measure 150) ;;
  nether1) args=(--count 0 --flyers 1 --fly-dim minecraft:the_nether --fly-center 2000,0 --fly-heading 1,0 --fly-altitude 180 --warmup 20 --measure 150) ;;
  ow8) args=(--count 0 --flyers 8 --fly-dim minecraft:overworld --fly-center 9000,0 --fly-heading 1,0 --fly-altitude 150 --warmup 20 --measure 150) ;;
  surv50) args=(--count 50 --warmup 60 --measure 180) ;;
  *) echo "unknown case $case_"; exit 2 ;;
esac
"$bin/survival_bench" "${common[@]}" "${args[@]}" "$@"
rm -rf "$world"
