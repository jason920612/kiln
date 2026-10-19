#!/bin/bash
# on the VM (via ~/kt wp82-gaps bash tools/wp82/verify80.sh): the final verification, three steps in a row, logs in /tmp/wp82_v{1,2,3}.txt
set +e
echo "start $(date +%T)" > /tmp/wp82_status.txt
cargo test --release --workspace --no-fail-fast > /tmp/wp82_v1.txt 2>&1
echo "v1 (no datapack) done $(date +%T)" >> /tmp/wp82_status.txt
KILN_DATAPACK=/home/test/kiln/work/generated cargo test --release --workspace --no-fail-fast > /tmp/wp82_v2.txt 2>&1
echo "v2 (datapack) done $(date +%T)" >> /tmp/wp82_status.txt
python3 tools/parity_suites.py --skip-workspace > /tmp/wp82_v3.txt 2>&1
echo "v3 (parity suites) done $(date +%T)" >> /tmp/wp82_status.txt
cargo check --workspace --all-targets > /tmp/wp82_check.txt 2>&1
echo "check done $(date +%T)" >> /tmp/wp82_status.txt
