#!/bin/bash
# On the VM: a second real 26.3 client ("Obs", agent port 26000, tiny view, 5 fps) that only watches what the
# server sends to the other players. usage: vm_observer61.sh [stop]; KILN_CLIENT_PORT picks the server.
pkill -f "mc-client/game2" 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
G=~/mc-client/game2
mkdir -p $G
[ -d $G/natives ] || cp -r ~/mc-client/game/natives $G/natives
sed -e 's/^maxFps:.*/maxFps:5/' -e 's/^renderDistance:.*/renderDistance:2/' -e 's/^simulationDistance:.*/simulationDistance:5/' ~/mc-client/game/options.txt > $G/options.txt
export DISPLAY=:99 ALSOFT_DRIVERS=null LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11
cd ~/mc-client
setsid nohup java -Xmx1500m -XX:+UseG1GC --enable-native-access=ALL-UNNAMED --add-exports java.base/jdk.internal.misc=ALL-UNNAMED \
  -javaagent:$HOME/wp63/clientctl.jar -Dwp63.port=26000 \
  -Djava.library.path=$G/natives/java -Djna.tmpdir=$G/natives/jna \
  -Dorg.lwjgl.system.SharedLibraryExtractPath=$G/natives/lwjgl -Dio.netty.native.workdir=$G/natives/netty \
  -cp "$(cat ~/mc-client/classpath.txt)" net.minecraft.client.main.Main \
  --username Obs --version 26.3 --gameDir $G --assetsDir ~/mc-client/assets --assetIndex 34 \
  --uuid 00000000000000000000000000000001 --accessToken 0 --userType legacy --versionType release \
  --width 480 --height 270 --quickPlayMultiplayer localhost:${KILN_CLIENT_PORT:-25585} > $G/stdout.log 2>&1 < /dev/null &
echo started observer
