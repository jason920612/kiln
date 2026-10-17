from lock import *
import threading, sys
sys.argv=["x"]
exec(open("p74_farm.py").read().split("# ------------------------------------------------------------------ scenarios")[0].replace("from lock import *",""))
def go(t):
    print(t, ask(t, "data get block -1293 66 735 Command"))
    print(t, ask(t, "kill @e[type=!player]"))
    sprint(t, 400)
    print(t, "witches", kill_count(t, "@e[type=minecraft:witch]"), "inbox", kill_count(t, "@e[type=minecraft:witch,x=-1281,y=58,z=735,dx=10,dy=14,dz=10]"))
    print(t, ask(t, "execute as @e[type=!player,type=!item] run say E").count("E"))
    print(t, ask(t, "gamerule spawn_monsters"), ask(t, "time query daytime"))
ths=[threading.Thread(target=go,args=(t,)) for t in "KV"]
[x.start() for x in ths]; [x.join() for x in ths]
