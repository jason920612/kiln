#!/bin/bash
# wp82 (on the VM): builds ~/wp82/agent82.jar (a java agent for the vanilla 26.3 server logging the end gateway's exit search) from tools/wp82/agent/W82*.java
set -e
W=${KILN_WORK:-$HOME/kiln/work}
CP=$(ls $W/versions/*/server-*.jar $(find $W/libraries -name '*.jar') | tr '\n' ':')
D=$(dirname "$0"); O=$HOME/wp82/agent-build; rm -rf $O; mkdir -p $O
javac -d $O -cp "$CP" $D/W82Agent.java $D/W82Hook.java
printf 'Premain-Class: w82.W82Agent\nCan-Retransform-Classes: true\n' > $O/manifest.txt
jar cfm $HOME/wp82/agent82.jar $O/manifest.txt -C $O w82
echo built $HOME/wp82/agent82.jar
