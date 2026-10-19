from lab84 import *
X0, Y, Z0 = 4000, 100, 4000
OB("forceload add 3980 3980 4020 4020")
for t in "KV":
    print(t, wait_loaded(t, X0, Y - 1, Z0, secs=90))
OB("fill 3984 99 3984 4016 99 4016 minecraft:stone")
OB("summon minecraft:zombie 4000 100 4000 {Tags:[\"j\"]}")
time.sleep(0.5)
for t in "KV":
    print(t, repr(ask(t, "execute in minecraft:overworld run tag @e[type=minecraft:zombie,tag=j,x=4000,y=100,z=4000,distance=..60] add jc", wait=0.5))[-200:])
    print(t, repr(ask(t, "execute in minecraft:overworld run kill @e[type=minecraft:zombie,tag=jc]", wait=0.5))[-200:])
