#!/bin/bash
# wp87 (on the VM): builds ~/wp87/agent87.jar (a java agent for the vanilla 26.3 server logging the wither's side-head target picks) from tools/wp87/agent/W87*.java
set -e
W=${KILN_WORK:-$HOME/kiln/work}
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
D=$(cd "$(dirname "$0")" && pwd); O=$HOME/wp87/agent-build; rm -rf $O; mkdir -p $O
javac -d $O -cp "$CP" $D/W87Agent.java $D/W87Hook.java
printf 'Premain-Class: w87.W87Agent\nCan-Retransform-Classes: true\n' > $O/manifest.txt
jar cfm $HOME/wp87/agent87.jar $O/manifest.txt -C $O w87
echo built $HOME/wp87/agent87.jar
