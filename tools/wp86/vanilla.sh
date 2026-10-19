#!/bin/bash
# wp86: a vanilla 26.3 reference server on the VM in ~/wp86/van (a fresh directory per start unless KEEP=1).
# usage: vanilla.sh start [props-file]   (props-file is copied to server.properties; eula=true is written)
#        vanilla.sh stop
# Pid in ~/wp86/van/java.pid, fifo keeper pid in ~/wp86/van/keeper.pid, console fifo ~/wp86/van/console, log server.log.
D=~/wp86/van
mkdir -p $D && cd $D
stop() {
  if [ -f java.pid ] && kill -0 "$(cat java.pid)" 2>/dev/null; then
    (exec 3<>console; echo stop >&3) 2>/dev/null
    for i in $(seq 1 30); do kill -0 "$(cat java.pid)" 2>/dev/null || break; sleep 1; done
    kill -0 "$(cat java.pid)" 2>/dev/null && kill "$(cat java.pid)"
  fi
  [ -f keeper.pid ] && kill "$(cat keeper.pid)" 2>/dev/null
  rm -f java.pid keeper.pid
}
case "$1" in
  stop) stop; exit 0;;
  start)
    stop
    if [ -z "$KEEP" ]; then rm -rf world logs server.properties eula.txt banned-*.json ops.json whitelist.json usercache.json server-icon.png; fi
    [ -d libraries ] || cp -r ~/wp86/v0/libraries ~/wp86/v0/versions .
    echo "eula=true" > eula.txt
    [ -n "$2" ] && cp "$2" server.properties
    rm -f console; mkfifo console
    setsid sleep 100000000 > console 2>/dev/null < /dev/null &
    echo $! > keeper.pid
    setsid nohup java -Xmx2G -jar ~/kiln/work/server.jar nogui < console > server.log 2>&1 &
    echo $! > java.pid
    echo "started $(cat java.pid)"
    ;;
esac
