#!/bin/bash
# on the VM: stop both clients and start them again in split mode (actor 25999 on Kiln, second client 26000 on vanilla)
pkill -f "mc-client/[g]ame"
sleep 3
cd ~/wp80 && bash switch.sh K split
