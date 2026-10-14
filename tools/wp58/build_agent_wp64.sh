#!/bin/bash
# On the VM: compile tools/wp58/ClientCtl.java against ~/mc-client into ~/wp64/clientctl.jar
set -e
SRC=${1:-$HOME/wt/wp64-wp63-gaps/tools/wp58/ClientCtl.java}
OUT=$HOME/wp64/agent; rm -rf $OUT; mkdir -p $OUT
javac -proc:none -d $OUT -cp "$(cat $HOME/mc-client/classpath.txt)" $SRC
echo "Premain-Class: wp58.ClientCtl" > $OUT/MANIFEST.MF
jar cfm $HOME/wp64/clientctl.jar $OUT/MANIFEST.MF -C $OUT wp58
echo built $HOME/wp64/clientctl.jar
