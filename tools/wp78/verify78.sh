#!/bin/bash
# on the VM (via ~/kt wp78-playthrough9 bash tools/wp78/verify78.sh): the final verification, three steps in a row, logs in /tmp/wp78_v{1,2,3}.txt
set +e
echo "start $(date +%T)" > /tmp/wp78_status.txt
cargo test --release --workspace --no-fail-fast > /tmp/wp78_v1.txt 2>&1
echo "v1 (no datapack) done $(date +%T)" >> /tmp/wp78_status.txt
KILN_DATAPACK=/home/test/kiln/work/generated cargo test --release --workspace --no-fail-fast > /tmp/wp78_v2.txt 2>&1
echo "v2 (datapack) done $(date +%T)" >> /tmp/wp78_status.txt
python3 tools/parity_suites.py --skip-workspace > /tmp/wp78_v3.txt 2>&1
echo "v3 (parity suites) done $(date +%T)" >> /tmp/wp78_status.txt
