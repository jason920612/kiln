#!/bin/bash
# wp86: a Kiln server on the VM in ~/wp86/kil (fresh directory per start unless KEEP=1).
# usage: kiln.sh start [props-file]   (props-file copied to server.properties; eula=true written unless NOEULA=1)
#        kiln.sh stop
# env passed through to the server: anything exported (KILN_* ...). Binary: $KILN_BIN or the wp86 checkout's release build.
D=~/wp86/kil
mkdir -p $D && cd $D
stop() {
  if [ -f kiln.pid ] && kill -0 "$(cat kiln.pid)" 2>/dev/null; then
    (exec 3<>console; echo stop >&3) 2>/dev/null
    for i in $(seq 1 30); do kill -0 "$(cat kiln.pid)" 2>/dev/null || break; sleep 1; done
    kill -0 "$(cat kiln.pid)" 2>/dev/null && kill "$(cat kiln.pid)"
  fi
  [ -f keeper.pid ] && kill "$(cat keeper.pid)" 2>/dev/null
  rm -f kiln.pid keeper.pid
}
case "$1" in
  stop) stop; exit 0;;
  start)
    stop
    if [ -z "$KEEP" ]; then rm -rf world logs server.properties eula.txt banned-*.json ops.json whitelist.json usercache.json server-icon.png; fi
    [ -z "$NOEULA" ] && echo "eula=true" > eula.txt
    [ -n "$2" ] && cp "$2" server.properties
    rm -f console; mkfifo console
    setsid sleep 100000000 > console 2>/dev/null < /dev/null &
    echo $! > keeper.pid
    BIN=${KILN_BIN:-$HOME/wt/wp86-server-ops/target/release/kiln}
    cp "$BIN" ./kiln-run86
    setsid nohup ./kiln-run86 $KILN_ARGS < console > server.log 2>&1 &
    echo $! > kiln.pid
    echo "started $(cat kiln.pid)"
    ;;
esac
