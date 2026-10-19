#!/bin/bash
# wp85 (on the VM, from the checkout, through ~/kt): the workspace tests without the Mojang data (the work link moved away,
# KILN_WORK and KILN_DATAPACK unset): what a clean checkout runs.
#   ~/kt wp85-worldgen-packs bash tools/wp85/nodata85.sh [cargo test arguments]
cd "$(dirname "$0")/../.."
unset KILN_WORK KILN_DATAPACK
mv work work.off
trap 'mv work.off work' EXIT
cargo test --release --workspace "$@"
