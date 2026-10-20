#!/bin/bash
# wp87 (on the VM): leash87.sh   vanilla's lead pulls (tools/wp87/LeashQuad.java) -> ~/kiln/work/wp87/leash.txt
set -e
W=${KILN_WORK:-$HOME/kiln/work}
OUT=$W/wp87
mkdir -p $OUT/leash-server
cd $OUT/leash-server
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
java -Xmx2g --add-opens java.base/java.lang=ALL-UNNAMED -cp "$CP" $HOME/wt/wp87-gaps/tools/wp87/LeashQuad.java $OUT/leash.txt 2>&1 | grep -v "^\[\|WARN" | tail -30
