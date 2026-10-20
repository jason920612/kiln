#!/bin/bash
# wp86 mgmt: push the branch to the VM and run a cargo command in its checkout, e.g.
#   tools/wp86/mgmt/vm.sh "check -p kiln-server --all-targets"
#   tools/wp86/mgmt/vm.sh "test --release -p kiln-server mgmt::"
# Output (errors and warnings, or the test summary) is filtered to what matters. Jobs: 2.
cd "$(dirname "$0")/../../.."
git push -q vm wp86-mgmt 2>&1 | tail -1
ssh -o BatchMode=yes test@192.168.1.169 "free -m | sed -n 2p; cd ~ && CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=${RUST_TEST_THREADS:-6} ~/kt wp86-mgmt cargo $1 2>&1" | grep -v "^\s*\(Compiling\|Checking\|Fresh\|Finished\|Running\|Downloaded\|Downloading\|Updating\|Locking\|Adding\)" 
