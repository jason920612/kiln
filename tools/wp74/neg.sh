#!/bin/bash
# on the VM: neg.sh "sed-expression-file-pairs" ...  negative check of a fix: neg.sh 'FILE|PATTERN' ... -- cargo test args
# deletes every line of FILE that contains PATTERN (in the ~/wt/wp74-playthrough8 checkout), runs the cargo command, restores the files with git checkout.
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} KILN_WORK=$HOME/kiln/work
cd ~/wt/wp74-playthrough8 || exit 1
edits=()
while [ "$1" != "--" ]; do edits+=("$1"); shift; done
shift
for e in "${edits[@]}"; do f="${e%%|*}"; p="${e#*|}"; sed -i "/$p/d" "$f"; done
"$@"
rc=$?
for e in "${edits[@]}"; do git checkout -q -- "${e%%|*}"; done
echo "NEG exit $rc"
