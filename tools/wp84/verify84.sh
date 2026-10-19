#!/bin/bash
# on the VM (via ~/kt wp84-playthrough11 bash tools/wp84/verify84.sh): the final verification, logs in ~/wp84/v84_*.txt, status in ~/wp84/v84_status.txt
set +e
S=$HOME/wp84/v84_status.txt
echo "start $(date +%T)" > $S
cargo check --workspace --all-targets > $HOME/wp84/v84_check.txt 2>&1
echo "check done $(date +%T) exit $?; warnings: $(grep -c '^warning' $HOME/wp84/v84_check.txt)" >> $S
cargo test --release --workspace --no-fail-fast > $HOME/wp84/v84_v1.txt 2>&1
echo "v1 (no datapack) done $(date +%T)" >> $S
KILN_DATAPACK=/home/test/kiln/work/generated cargo test --release --workspace --no-fail-fast > $HOME/wp84/v84_v2.txt 2>&1
echo "v2 (datapack) done $(date +%T)" >> $S
python3 tools/parity_suites.py --skip-workspace > $HOME/wp84/v84_v3.txt 2>&1
echo "v3 (parity suites) done $(date +%T)" >> $S
echo done > $HOME/wp84/v84_done
