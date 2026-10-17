#!/bin/bash
# wp77 (on the VM): bot.sh K|V|stop [prefix] [count] [behavior]   idle kiln-bots against Kiln (25599) or vanilla (25596); names <prefix>0...
who=$1; prefix=${2:-W}; count=${3:-1}; beh=${4:-idle}
if [ "$who" = stop ]; then pkill -f "wt/wp77-gaps/target/release/kiln-bot"; exit 0; fi
port=25599; [ $who = V ] && port=25596
nohup $HOME/wt/wp77-gaps/target/release/kiln-bot --addr 127.0.0.1:$port --count $count --rate 5 --behavior $beh --duration 100000 --view-distance ${VIEW:-8} --name-prefix $prefix > $HOME/wp77/bot_$who.log 2>&1 < /dev/null &
echo bots $who $!
