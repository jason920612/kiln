#!/bin/bash
# wp87 (on the VM): mace87.sh [filter]   vanilla's smash attacks from real falls (tools/wp87/MaceFall.java) -> ~/kiln/work/wp87/mace.jsonl
# The harness' server binds 127.0.0.1:25597 in a scratch directory.
set -e
W=${KILN_WORK:-$HOME/kiln/work}
OUT=$W/wp87
mkdir -p $OUT $OUT/mace-server
cd $OUT/mace-server
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
java -Xmx2g --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" $HOME/wt/wp87-gaps/tools/wp87/MaceFall.java $OUT/mace.jsonl $1 2>&1 | grep -v "^\[" | tail -30
