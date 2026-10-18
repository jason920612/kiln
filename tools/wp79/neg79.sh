#!/bin/bash
# wp79 (on the VM, in ~/wt/wp79-gaps): neg79.sh NAME FILE PERL_EDIT CARGO_TEST_ARGS...
# Applies one perl edit to FILE (the fix switched off), runs the test, which must FAIL ("failed (good)"), then restores the file with git checkout.
name=$1; file=$2; edit=$3; shift 3
export PATH="$HOME/.cargo/bin:$PATH" KILN_WORK="$HOME/kiln/work" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cd "$HOME/wt/wp79-gaps" || exit 2
git checkout -q -- "$file"
perl -0pi -e "$edit" "$file"
if git diff --quiet -- "$file"; then echo "NEG $name: the edit changed nothing"; exit 3; fi
if cargo test --release "$@" > "$HOME/wp79/neg_$name.log" 2>&1; then
  echo "NEG $name: PASSED without the fix (bad)"; r=1
else
  if grep -q "error\[E\|could not compile" "$HOME/wp79/neg_$name.log"; then echo "NEG $name: does not compile (invalid case)"; r=3; else echo "NEG $name: failed (good)"; r=0; fi
fi
git checkout -q -- "$file"
exit $r
