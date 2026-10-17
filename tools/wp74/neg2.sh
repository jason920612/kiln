#!/bin/bash
# on the VM: neg2.sh REV FILE... -- cargo test args : the files as they were at REV (the fix removed), run, restore
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} KILN_WORK=$HOME/kiln/work
cd ~/wt/wp74-playthrough8 || exit 1
rev="$1"; shift
files=()
while [ "$1" != "--" ]; do files+=("$1"); shift; done
shift
git checkout -q "$rev" -- "${files[@]}"
"$@" 2>&1 | tail -${TAILN:-15}
rc=${PIPESTATUS[0]}
git checkout -q HEAD -- "${files[@]}"
echo "NEG exit $rc"
