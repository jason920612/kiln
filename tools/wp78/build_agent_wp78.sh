#!/bin/bash
# On the VM: compile tools/wp58/ClientCtl.java against ~/mc-client into ~/wp78/clientctl.jar
set -e
SRC=${1:-$HOME/wt/wp78-playthrough9/tools/wp58/ClientCtl.java}
OUT=$HOME/wp78/agent; rm -rf $OUT; mkdir -p $OUT
javac -proc:none -d $OUT -cp "$(cat $HOME/mc-client/classpath.txt)" $SRC
echo "Premain-Class: wp58.ClientCtl" > $OUT/MANIFEST.MF
jar cfm $HOME/wp78/clientctl.jar $OUT/MANIFEST.MF -C $OUT wp58
echo built $HOME/wp78/clientctl.jar
