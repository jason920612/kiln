#!/bin/bash
# wp77 (on the VM): builds ~/wp77/agent77.jar (a java agent for the vanilla 26.3 server) from tools/wp77/agent/W72*.java (the wp74 agent plus per-height in-box counts)
set -e
W=${KILN_WORK:-$HOME/kiln/work}
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
D=$(dirname "$0"); O=$HOME/wp77/agent-build; rm -rf $O; mkdir -p $O
javac -d $O -cp "$CP" $D/W72Agent.java $D/W72Hook.java
printf 'Premain-Class: w72.W72Agent\nCan-Retransform-Classes: true\n' > $O/manifest.txt
jar cfm $HOME/wp77/agent77.jar $O/manifest.txt -C $O w72
echo built $HOME/wp77/agent77.jar
