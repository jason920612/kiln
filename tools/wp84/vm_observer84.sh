#!/bin/bash
# On the VM: a second real 26.3 client ("Obs", agent port 26000, tiny view, 5 fps) that only watches what the
# server sends to the other players. usage: vm_observer84.sh [stop]; KILN_CLIENT_PORT picks the server.
pkill -f "mc-client/game2" 2>/dev/null
sleep 1
[ "$1" = stop ] && exit 0
G=~/mc-client/game2
mkdir -p $G
[ -d $G/natives ] || cp -r ~/mc-client/game/natives $G/natives
sed -e "s/^maxFps:.*/maxFps:${OBS_FPS:-8}/" -e "s/^renderDistance:.*/renderDistance:${OBS_RD:-4}/" -e "s/^simulationDistance:.*/simulationDistance:${OBS_SD:-6}/" ~/mc-client/game/options.txt > $G/options.txt
export DISPLAY=:99 ALSOFT_DRIVERS=null LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11
cd ~/mc-client
setsid nohup java -Xmx1500m -XX:+UseG1GC --enable-native-access=ALL-UNNAMED --add-exports java.base/jdk.internal.misc=ALL-UNNAMED \
  -javaagent:$HOME/wp84/clientctl.jar -Dwp61.port=26000 \
  -Djava.library.path=$G/natives/java -Djna.tmpdir=$G/natives/jna \
  -Dorg.lwjgl.system.SharedLibraryExtractPath=$G/natives/lwjgl -Dio.netty.native.workdir=$G/natives/netty \
  -cp "$(cat ~/mc-client/classpath.txt)" net.minecraft.client.main.Main \
  --username ${OBS_NAME:-Obs} --version 26.3 --gameDir $G --assetsDir ~/mc-client/assets --assetIndex 34 \
  --uuid 00000000000000000000000000000001 --accessToken 0 --userType legacy --versionType release \
  --width 480 --height 270 --quickPlayMultiplayer localhost:${KILN_CLIENT_PORT:-25589} > $G/stdout.log 2>&1 < /dev/null &
echo started observer
