#!/bin/bash
# wp85 (on the VM, from the checkout): what vanilla sends a client for the wp85 test packs (dimension types, biomes...) against what
# Kiln sends: the checker of wp83 (tools/wp83/CheckRegistries.java: vanilla's own loader, network codecs and tags) on packs of tools/wp85/packs.
#   tools/wp85/check_registries85.sh [pack ...]        (default: all the wp85 packs that do not replace each other's files)
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
packs=${@:-kiln85_desert kiln85_crystal kiln85_structure kiln85_dims kiln85_nether}
W=${KILN_WORK:-$HOME/kiln/work}
D=$(mktemp -d)
mkdir -p $D/world/datapacks
for p in $packs; do cp -r tools/wp85/packs/$p $D/world/datapacks/$(basename $p); done
KILN_DATAPACK=$W/generated cargo run -q --release --example wp83_dump -p kiln-sim -- $D/world > $D/kiln.txt 2> $D/kiln.err || { cat $D/kiln.err; exit 1; }
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
ROOT=$PWD
cd $W
set +e
java -Xss8m -cp "$CP" $ROOT/tools/wp83/CheckRegistries.java $D/world/datapacks $D/kiln.txt $D/vanilla.txt
status=$?
echo "dumps: $D (kiln.txt, vanilla.txt)"
exit $status
