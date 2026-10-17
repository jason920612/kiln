from lock import *
both_("kill @e[type=!player]"); time.sleep(0.5)
both_("summon minecraft:witch -1276 64 739 {NoAI:1b,PersistenceRequired:1b}")
both_("summon minecraft:zombie -1276 64 750 {NoAI:1b}")
both_("summon minecraft:witch -1276 64 750 {NoAI:1b}")
time.sleep(2)
for t in "KV":
    print(t, ask(t, "execute if entity @e[type=witch,x=-1281,y=58,z=735,dx=10,dy=14,dz=10]"), ask(t, "execute if entity @e[type=zombie]"),  ask(t, "execute if entity @e[type=witch,x=-1281,y=58,z=746,dx=10,dy=14,dz=10]"))
both_("kill @e[type=!player]")
