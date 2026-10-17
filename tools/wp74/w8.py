from lock import *
import threading, sys
TYPES = ["zombie","skeleton","creeper","spider","witch","enderman","bat","slime","cow","sheep","pig","chicken","item","squid","cat","bogged","zombie_villager","cave_spider","frog","tadpole"]
TOD = sys.argv[1:] or ["6000", "18000"]
for tod in TOD:
    both_(f"time set {tod}")
    both_("kill @e[type=!player]")
    time.sleep(1)
    def go(t):
        sprint(t, 300)
        time.sleep(1)
        res = {ty: kill_count(t, f"@e[type=minecraft:{ty}]") for ty in TYPES}
        print(tod, t, {k: v for k, v in res.items() if v}, flush=True)
    ths=[threading.Thread(target=go,args=(t,)) for t in "KV"]
    [x.start() for x in ths]; [x.join() for x in ths]
