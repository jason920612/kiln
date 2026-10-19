#!/bin/bash
# wp83 (on the VM): bench83.sh base|new [packs] [extra survival_bench args]
#   the survival benchmark (survival bots on vanilla noise terrain, server view and simulation distance 10, Kiln on port 25589) against the build of
#   main (`base`, ~/wt/wp83-base: ba0f822c) or of this branch (`new`, ~/wt/wp83-datapacks). `packs` (default none): test packs of tools/wp83/packs to
#   put in the world's datapacks before it starts (`kiln_wp83`): the registries and tags of a pack in the hot paths.
#   Results in ~/wp83/out/<tag>*; the numbers that matter are the tick time (mean, p50, p99, max), the bots' join and chunk latencies.
#   Build both first: cargo build --release -p kiln-server -p kiln-bot --bins --examples (CARGO_BUILD_JOBS=3), in each checkout.
which=$1; packs=${2:-}; shift $(( $# < 2 ? $# : 2 ))
case $which in base) dir=$HOME/wt/wp83-base;; new) dir=$HOME/wt/wp83-datapacks;; *) echo "base|new [packs]"; exit 2;; esac
tag="$which-${packs:-nopack}-$(date +%H%M)"
out="$HOME/wp83/out"; mkdir -p "$out"
world="/tmp/wp83-world-$tag"; rm -rf "$world"
if [ -n "$packs" ]; then
  mkdir -p "$world/datapacks"
  for p in $packs; do cp -r "$HOME/wt/wp83-datapacks/tools/wp83/packs/$p" "$world/datapacks/$p"; done
fi
$HOME/wt/wp83-datapacks/target/release/examples/survival_bench --server "$dir/target/release/kiln" --bot "$HOME/wt/wp83-datapacks/target/release/kiln-bot" \
  --datapack /home/test/kiln/work/generated --world "$world" --phase gen --port 25589 --slow-print 10 --out "$out" --tag "$tag" \
  --count 50 --warmup 60 --measure 180 --view-distance 10 --simulation-distance 10 "$@"
rm -rf "$world"
