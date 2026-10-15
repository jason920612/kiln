#!/bin/bash
# On the VM: the real 26.3 client (KilnTest, wp58 driver agent) for wp67's elytra flight, connecting to localhost:25597.
# While another session uses Xvfb :99 and ~/mc-client, run it from the copy ~/mc-client67 on :67 with agent port 26067:
#   KILN_CLIENT_DIR=~/mc-client67 KILN_DISPLAY=:67 KILN_AGENT_PORT=26067 vm_client67.sh
# usage: vm_client67.sh [stop]
D=${KILN_CLIENT_DIR:-$HOME/mc-client67}
DISP=${KILN_DISPLAY:-:67}
PORT=${KILN_AGENT_PORT:-26067}
pkill -f "$D/game/" 2>/dev/null
sleep 1
[ "$1" = stop ] && { pkill -f "Xvfb $DISP" 2>/dev/null; exit 0; }
cd "$D"
pgrep -f "Xvfb $DISP" >/dev/null || (setsid nohup Xvfb $DISP -screen 0 960x540x24 +extension GLX +render -noreset > "$D/xvfb.log" 2>&1 < /dev/null &)
sleep 1
G=$D/game
mkdir -p $G/natives/{java,jna,lwjgl,netty}
[ -f $G/options.txt ] || cat > $G/options.txt <<OPT
onboardAccessibility:false
skipMultiplayerWarning:true
joinedFirstServer:true
pauseOnLostFocus:false
renderDistance:8
soundCategory_master:0.0
fullscreen:false
tutorialStep:none
OPT
export DISPLAY=$DISP ALSOFT_DRIVERS=null LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11
setsid nohup java -Xmx2G -XX:+UseG1GC --enable-native-access=ALL-UNNAMED --add-exports java.base/jdk.internal.misc=ALL-UNNAMED \
  -javaagent:$HOME/wp65/clientctl.jar -Dwp61.port=$PORT \
  -Djava.library.path=$G/natives/java -Djna.tmpdir=$G/natives/jna \
  -Dorg.lwjgl.system.SharedLibraryExtractPath=$G/natives/lwjgl -Dio.netty.native.workdir=$G/natives/netty \
  -cp "$(sed "s|$HOME/mc-client/|$D/|g" $D/classpath.txt)" net.minecraft.client.main.Main \
  --username KilnTest --version 26.3 --gameDir $G --assetsDir $D/assets --assetIndex 34 \
  --uuid 00000000000000000000000000000000 --accessToken 0 --userType legacy --versionType release \
  --width 960 --height 540 --quickPlayMultiplayer localhost:${KILN_CLIENT_PORT:-25597} > $G/stdout.log 2>&1 < /dev/null &
echo started
