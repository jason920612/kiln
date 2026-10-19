#!/bin/bash
# on the laptop: push.sh   copy tools/wp82 to the VM's ~/wp82 (after ~/kt reset the worktree)
ssh -o BatchMode=yes test@192.168.1.169 "mkdir -p ~/wp82" && scp -q -o BatchMode=yes "$(dirname "$0")"/*.py "$(dirname "$0")"/*.sh test@192.168.1.169:wp82/
