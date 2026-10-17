#!/bin/bash
# on the VM: final verification of wp74 (logs in ~/final74_*.txt, ~/final74_done at the end)
export PATH=$HOME/.cargo/bin:$PATH CARGO_BUILD_JOBS=4
rm -f ~/final74_done
~/kt wp74-playthrough8 bash -c "
echo ==A nodatapack; cargo test --workspace --release --no-fail-fast 2>&1 | grep -E \"^test result|FAILED|failed|panicked|^error\" > ~/final74_A.txt
echo ==B datapack; KILN_DATAPACK=/home/test/kiln/work/generated cargo test --workspace --release --no-fail-fast 2>&1 | grep -E \"^test result|FAILED|failed|panicked|^error\" > ~/final74_B.txt
echo ==C; cargo test --release -p kiln-sim --test determinism 2>&1 | grep -E \"^test result|FAILED|panicked\" > ~/final74_C.txt
echo ==D; cargo test --release -p kiln-entity --test mob_parity 2>&1 | grep -E \"^test result|FAILED|panicked\" > ~/final74_D.txt
echo ==E; python3 tools/parity_suites.py --skip-workspace > ~/final74_E.txt 2>&1
"
echo FINALDONE > ~/final74_done
