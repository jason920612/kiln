#!/bin/bash
# wp76 final verification on the VM: workspace tests without and with the datapack (the latter checking every item the
# area searches pass over, KILN_CHECK_PREFILTER=1), the parity suites, command_diff (Kiln 25590, vanilla 25597).
# Logs in ~/wp76/final/; ~/wp76/final/done when finished.
export PATH=$HOME/.cargo/bin:$PATH KILN_WORK=$HOME/kiln/work CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-5}
F=~/wp76/final
mkdir -p $F && rm -f $F/done
~/kt wp76-items-perf true
cd ~/wt/wp76-items-perf
git log --oneline -1 > $F/head.txt
(unset KILN_DATAPACK; cargo test --release --workspace --no-fail-fast > $F/ws_nodata.log 2>&1; echo "EXIT $?" >> $F/ws_nodata.log)
(export KILN_DATAPACK=$HOME/kiln/work/generated KILN_CHECK_PREFILTER=1; cargo test --release --workspace --no-fail-fast > $F/ws_data.log 2>&1; echo "EXIT $?" >> $F/ws_data.log)
mkdir -p $F/suites
python3 tools/parity_suites.py --skip-workspace --out $F/suites > $F/suites.log 2>&1
echo "SUITES-EXIT $?" >> $F/suites.log
cargo build --release -p kiln-server -p kiln-bot > $F/build.log 2>&1
echo "BUILD-EXIT $?" >> $F/build.log
python3 tools/command_diff.py --kiln-exe target/release/kiln --bot-exe target/release/kiln-bot --kiln-port 25590 --vanilla-port 25597 > $F/cmd.log 2>&1
echo "CMD-EXIT $?" >> $F/cmd.log
echo ALLDONE > $F/done
