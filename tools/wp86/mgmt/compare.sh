#!/bin/bash
# wp86: diff two probe transcripts (vanilla, Kiln) without what depends on timing or is unordered in vanilla:
# the save/start/status notifications (autosave and the harness's start-up race), the game rule list (vanilla lists
# its rules in an identity-hash order that changes from run to run) and the join/leave chat lines of players.
# usage: compare.sh van.txt kil.txt
norm() {
  grep -v -E 'notification/server/(saving|saved|started|status)"|^(<-|\[obs\]) \{"jsonrpc":"2.0","id":[0-9]+,"result":\[\{"type":"(boolean|integer)"|^game> .* 124 |^(-- the server is gone|<- <closed>|Traceback|  File|    |EOFError|During|ConnectionReset|$)' "$1"
}
diff <(norm "$1") <(norm "$2")
