#!/bin/bash
# wp83 (on the VM, from the checkout): what vanilla sends a client for the test packs against what Kiln sends.
#   tools/wp83/check_registries.sh [pack ...]        (default: kiln_wp83; packs are directories of tools/wp83/packs)
# Kiln's side is the `wp83_dump` example (a Sim on a world holding the packs), vanilla's is tools/wp83/CheckRegistries.java
# (WorldLoader + RegistrySynchronization + TagNetworkSerialization from the server jar).
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
packs=${@:-kiln_wp83}
W=${KILN_WORK:-$HOME/kiln/work}
D=$(mktemp -d)
mkdir -p $D/world/datapacks
for p in $packs; do cp -r tools/wp83/packs/$p $D/world/datapacks/$p; done
KILN_DATAPACK=$W/generated cargo run -q --release --example wp83_dump -p kiln-sim -- $D/world > $D/kiln.txt 2> $D/kiln.err || { cat $D/kiln.err; exit 1; }
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
ROOT=$PWD
cd $W
set +e
java -Xss8m -cp "$CP" $ROOT/tools/wp83/CheckRegistries.java $D/world/datapacks $D/kiln.txt $D/vanilla.txt
status=$?
echo "dumps: $D (kiln.txt, vanilla.txt)"
exit $status
