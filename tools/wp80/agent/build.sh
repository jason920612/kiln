#!/bin/bash
# wp80: builds ~/wp80/agent80.jar (a java agent for the vanilla 26.3 server) from tools/wp80/agent/W72*.java
set -e
W=${KILN_WORK:-$HOME/kiln/work}
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
D=$(dirname "$0"); O=$HOME/wp80/agent-build78; rm -rf $O; mkdir -p $O
javac -d $O -cp "$CP" $D/W72Agent.java $D/W72Hook.java
printf 'Premain-Class: w72.W72Agent\nCan-Retransform-Classes: true\n' > $O/manifest.txt
jar cfm $HOME/wp80/agent80.jar $O/manifest.txt -C $O w72
echo built $HOME/wp80/agent80.jar
