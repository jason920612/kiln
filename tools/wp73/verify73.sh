#!/bin/bash
# wp73 verification on the VM (branch checked out by ~/kt in ~/wt/wp73-stalls): workspace tests without and with
# KILN_DATAPACK, the parity suites, command_diff (Kiln 25590, vanilla 25597). Logs in ~/wp73/verify/.
#   verify73.sh [steps...]   steps: ws ws_dp suites diff (default all)
set -u
out=~/wp73/verify; mkdir -p "$out"
steps=("${@:-ws ws_dp suites diff}")
export PATH="$HOME/.cargo/bin:$PATH" KILN_WORK="$HOME/kiln/work" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-6}"
cd ~/wt/wp73-stalls || exit 1
for s in ${steps[@]}; do
  case $s in
    ws) env -u KILN_DATAPACK cargo test --workspace --release --no-fail-fast > "$out/ws.log" 2>&1; echo "ws exit $?" >> "$out/summary" ;;
    ws_dp) KILN_DATAPACK=/home/test/kiln/work/generated cargo test --workspace --release --no-fail-fast > "$out/ws_dp.log" 2>&1; echo "ws_dp exit $?" >> "$out/summary" ;;
    suites) python3 tools/parity_suites.py --skip-workspace --out "$out/suites" > "$out/suites.log" 2>&1; echo "suites exit $?" >> "$out/summary" ;;
    diff) cargo build --release -p kiln-server -p kiln-bot > "$out/diff_build.log" 2>&1
          python3 tools/command_diff.py --kiln-port 25590 --vanilla-port 25597 > "$out/diff.log" 2>&1; echo "diff exit $?" >> "$out/summary" ;;
  esac
done
echo done >> "$out/summary"
