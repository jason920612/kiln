#!/bin/bash
# wp87 (on the VM): bot.sh K|V|stop [prefix] [count] [behavior]   idle kiln-bots against Kiln (25599) or vanilla (25596); names <prefix>0...; PIDs in ~/wp87/bot_<K|V>.pid
who=$1; prefix=${2:-W}; count=${3:-1}; beh=${4:-idle}
mkdir -p $HOME/wp87
if [ "$who" = stop ]; then for f in $HOME/wp87/bot_K.pid $HOME/wp87/bot_V.pid; do [ -f $f ] && kill $(cat $f) 2>/dev/null; rm -f $f; done; exit 0; fi
port=25599; [ $who = V ] && port=25596
nohup $HOME/wt/wp87-gaps/target/release/kiln-bot --addr 127.0.0.1:$port --count $count --rate 5 --behavior $beh --duration 100000 --view-distance ${VIEW:-8} --name-prefix $prefix > $HOME/wp87/bot_$who.log 2>&1 < /dev/null &
echo $! > $HOME/wp87/bot_$who.pid
echo bots $who $!
