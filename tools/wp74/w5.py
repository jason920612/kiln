from lock import *
for t in "KV":
    print(t, ask(t, "gamerule mob_drops")[-80:], ask(t, "execute if entity @e[type=item]")[-80:])
both_("summon minecraft:zombie -1276 64 750 {NoAI:1b,Health:1f,DeathLootTable:\"minecraft:entities/zombie\"}")
time.sleep(1)
both_("kill @e[type=zombie]")
time.sleep(1.5)
for t in "KV":
    print(t, ask(t, "execute if entity @e[type=item]")[-80:])
both_("summon minecraft:cow -1276 64 750")
time.sleep(1)
both_("kill @e[type=cow]")
time.sleep(1.5)
for t in "KV":
    print(t, ask(t, "execute if entity @e[type=item]")[-80:])
