from lock import *
import threading
both_("setblock -1293 66 735 air")
both_("kill @e[type=!player]")
time.sleep(1)
def go(t):
    sprint(t, 100)
    n_mob = kill_count(t, "@e[type=!player,type=!item]")
    time.sleep(1)
    print(t, "mobs killed", n_mob, "items", kill_count(t, "@e[type=item]"), flush=True)
    # same again but kill every tick through a command block
    w(t, 'setblock -1293 66 735 minecraft:repeating_command_block{auto:1b,TrackOutput:0b,Command:"kill @e[type=!player,type=!item]"}')
    time.sleep(1)
    sprint(t, 200)
    time.sleep(1)
    print(t, "with block: items", kill_count(t, "@e[type=item]"), flush=True)
    w(t, "setblock -1293 66 735 air")
ths=[threading.Thread(target=go,args=(t,)) for t in "KV"]
[x.start() for x in ths]; [x.join() for x in ths]
