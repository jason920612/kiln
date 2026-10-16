#!/bin/bash
# wp71: builds ~/w71/agent.jar (a java agent for the vanilla 26.3 server) from tools/wp71/W71*.java
set -e
W=${KILN_WORK:-$HOME/kiln/work}
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
D=$(dirname "$0"); O=$HOME/w71/agent-build; rm -rf $O; mkdir -p $O
javac -d $O -cp "$CP" $D/W71Agent.java $D/W71Hook.java
printf 'Premain-Class: w71.W71Agent\nCan-Retransform-Classes: true\n' > $O/manifest.txt
jar cfm $HOME/w71/agent.jar $O/manifest.txt -C $O w71
echo built $HOME/w71/agent.jar
