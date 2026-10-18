#!/bin/bash
# wp79 (on the VM): bench79.sh base|new BOT_VIEW [extra survival_bench args]
#   the wp76/wp73 survival benchmark (50 survival bots, server view and simulation distance 10, Kiln on port 25599) with the bots asking for render distance BOT_VIEW,
#   against the build of main (`base`, ~/wt/wp79-base) or of this branch (`new`, ~/wt/wp79-gaps). Results in ~/wp79/out/<tag>*.
which=$1; bv=$2; shift 2
case $which in base) dir=$HOME/wt/wp79-base;; new) dir=$HOME/wt/wp79-gaps;; *) echo "base|new"; exit 2;; esac
tag="$which-bv$bv-$(date +%H%M)"
out="$HOME/wp79/out"; mkdir -p "$out"
world="/tmp/wp79-world-$tag"; rm -rf "$world"
$HOME/wt/wp79-gaps/target/release/examples/survival_bench --server "$dir/target/release/kiln" --bot "$HOME/wt/wp79-gaps/target/release/kiln-bot" \
  --datapack /home/test/kiln/work/generated --world "$world" --phase gen --port 25599 --slow-print 10 --out "$out" --tag "$tag" \
  --count 50 --warmup 60 --measure 180 --view-distance 10 --simulation-distance 10 --bot-view-distance "$bv" "$@"
rm -rf "$world"
