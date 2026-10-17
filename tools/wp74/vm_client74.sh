#!/bin/bash
# On the VM: start (or restart) Xvfb :99 and the real 26.3 client in ~/mc-client, offline as KilnTest,
# connecting to the Kiln server on localhost:25589, with the wp58 driver agent loaded.
# usage: vm_client.sh [stop]
pkill -f "mc-client/game/" 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
cd ~/mc-client
pgrep -f "Xvfb :99" >/dev/null || (nohup Xvfb :99 -screen 0 960x540x24 +extension GLX +render -noreset > ~/wp58/xvfb.log 2>&1 &)
sleep 1
G=~/mc-client/game
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
export DISPLAY=:99 ALSOFT_DRIVERS=null LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11
setsid nohup java -Xmx2G -XX:+UseG1GC --enable-native-access=ALL-UNNAMED --add-exports java.base/jdk.internal.misc=ALL-UNNAMED \
  -javaagent:$HOME/wp74/clientctl.jar \
  -Djava.library.path=$G/natives/java -Djna.tmpdir=$G/natives/jna \
  -Dorg.lwjgl.system.SharedLibraryExtractPath=$G/natives/lwjgl -Dio.netty.native.workdir=$G/natives/netty \
  -cp "$(cat ~/mc-client/classpath.txt)" net.minecraft.client.main.Main \
  --username ${KILN_CLIENT_NAME:-KilnTest} --version 26.3 --gameDir $G --assetsDir ~/mc-client/assets --assetIndex 34 \
  --uuid 00000000000000000000000000000000 --accessToken 0 --userType legacy --versionType release \
  --width 960 --height 540 --quickPlayMultiplayer localhost:${KILN_CLIENT_PORT:-25589} > $G/stdout.log 2>&1 < /dev/null &
echo started
