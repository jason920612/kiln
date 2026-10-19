#!/bin/bash
# wp84 negative checks, on the VM: each fix switched off by a text edit, the test that belongs to it must fail (see neg84.py).
# usage: bash tools/wp84/neg84.sh [case-name-fragment ...]   (run in the VM checkout; takes a few minutes per case)
cd "$(dirname "$0")/../.." && export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-6}" KILN_WORK="${KILN_WORK:-$HOME/kiln/work}"
exec python3 tools/wp84/neg84.py "$@"
