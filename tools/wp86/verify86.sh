#!/bin/bash
# wp86 final verification (on the VM, from the checkout, through ~/kt), one step per output file in ~/wp86/v/:
#   nohup ~/kt wp86-server-ops bash tools/wp86/verify86.sh [steps...] > ~/wp86/v/all.out 2>&1 &
# steps (default: all, in this order): check test_data test_nodata test_default build parity cmpdiff1 cmpdiff2 cmpdiff3 plugins neg
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} RUST_TEST_THREADS=${RUST_TEST_THREADS:-6}
OUT=~/wp86/v
mkdir -p $OUT
steps="${*:-check test_data test_nodata test_default build parity cmpdiff1 cmpdiff2 cmpdiff3 plugins neg}"
for step in $steps; do
  echo "== $step $(date +%T)"
  case $step in
    check) cargo check --workspace --all-targets --release > $OUT/check.out 2>&1; grep -c "^warning" $OUT/check.out ;;
    test_data) KILN_DATAPACK=/home/test/kiln/work/generated cargo test --release --workspace --no-fail-fast > $OUT/test_data.out 2>&1 ;;
    test_nodata) bash tools/wp85/nodata85.sh --no-fail-fast > $OUT/test_nodata.out 2>&1 ;;
    test_default) KILN_DATAPACK=/home/test/kiln/work/generated env -u RUST_TEST_THREADS cargo test --release --workspace --no-fail-fast > $OUT/test_default.out 2>&1 ;;
    build) cargo build --release -p kiln-server -p kiln-bot > $OUT/build.out 2>&1 ;;
    parity) python3 tools/parity_suites.py --skip-workspace > $OUT/parity.out 2>&1 ;;
    cmpdiff1|cmpdiff2|cmpdiff3) KILN_DATAPACK=/home/test/kiln/work/generated python3 tools/command_diff.py --kiln-port 25589 --vanilla-port 25595 --kiln-exe target/release/kiln --bot-exe target/release/kiln-bot > $OUT/$step.out 2>&1 ;;
    plugins) cargo test --release -p kiln-plugin-host --test api --test api_compat --test api_property --test async_tasks --test examples --test ledger_property --test wit_freeze > $OUT/plugins.out 2>&1
      cargo test --release -p kiln-sim --test plugins --test plugin_api --test plugin_api_determinism >> $OUT/plugins.out 2>&1 ;;
    neg) bash tools/wp86/neg86.sh > $OUT/neg.out 2>&1 ;;
  esac
  echo "   exit $?"
  if [ -d work.off ] && [ ! -e work ]; then mv work.off work; fi
done
echo "== all done $(date +%T)"
