from lab84 import *
cl = clients()
def inv(c): return {k.split(":")[1]: v for k, v in c.inv().items() if k.split(":")[1] in ("dirt", "flint", "emerald", "porkchop", "diamond")}
OB("setblock 20 199 24 minecraft:dirt")
for t in "KV":
    w(t, "clear KilnTest"); w(t, "gamemode survival KilnTest"); w(t, "execute in minecraft:overworld run tp KilnTest 20.5 200 20.5")
time.sleep(2)
for tool in ("minecraft:iron_shovel", "minecraft:iron_pickaxe[enchantments={\"minecraft:silk_touch\":1}]"):
    for t in "KV":
        w(t, "clear KilnTest")
        w(t, f"execute in minecraft:overworld run loot give KilnTest mine 20 199 24 {tool}")
    time.sleep(1.5)
    print(tool[:30], {t: inv(cl[t]) for t in "KV"})
for t in "KV":
    w(t, "clear KilnTest")
    w(t, "execute in minecraft:overworld run loot give KilnTest loot minecraft:entities/pig")
time.sleep(1.5)
print("pig table", {t: inv(cl[t]) for t in "KV"})
for t in "KV":
    w(t, "clear KilnTest")
    w(t, "execute in minecraft:overworld run loot give KilnTest loot kw84:gift")
time.sleep(1.5)
print("gift table", {t: inv(cl[t]) for t in "KV"})
def ex(t): return nbt(t, "entity", "KilnTest", "foodExhaustionLevel")
for t in "KV": w(t, "effect give KilnTest minecraft:saturation 1 0 true")
time.sleep(1)
b = {t: ex(t) for t in "KV"}
for t in "KV": w(t, "execute in minecraft:overworld run damage KilnTest 3 kw84:laser")
time.sleep(0.5)
print("laser exhaustion", b, {t: ex(t) for t in "KV"})
