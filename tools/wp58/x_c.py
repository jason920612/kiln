from p68_sulfur import *
prep(); time.sleep(3)
eid = fresh_cube()
F("cubes on server before:", sv("execute store result score x _t run execute if entity @e[type=sulfur_cube]", show=False)[:100] or "?")
give(ME, "minecraft:bucket")
ok = use_on_entity(a, eid, "minecraft:bucket")
time.sleep(0.8)
F("after pickup: server count:", sv("kill @e[type=sulfur_cube,limit=0]", show=False)[:200], sv("execute if entity @e[type=sulfur_cube]", show=False)[:200], "client sees", len(ents_of(a, "sulfur_cube", 40)))
print(a.cmd("comps 0"))
