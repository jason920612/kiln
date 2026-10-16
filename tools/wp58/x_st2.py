from p68_ghast import *
kill_all_mobs()
sv("tp KilnTest 26.5 200 22.5", show=False); sv("tp Obs 40.5 200 5.5", show=False)
time.sleep(2)
sv("summon minecraft:happy_ghast 26.5 202 26.5 {Age:0,NoAI:1b}", show=False)
time.sleep(1.5)
g = ents_of(a, "happy_ghast", 40)[0]
eid = g["id"]
give("KilnTest", "minecraft:blue_harness", 1)
use_on_entity(a, eid, "minecraft:blue_harness", dy=1.5)
time.sleep(0.8)
ok = interact_ent(a, eid, button="right", tries=8, dy=1.5)
F("riding", ok, a.state(0)["vehicle"])
rec_start(a)
for i in range(6):
    time.sleep(0.4)
    F(i, "vehicle", a.state(0)["vehicle"], pos(a))
sv("data merge entity @e[type=happy_ghast,limit=1] {NoAI:0b}", show=False)
for i in range(10):
    time.sleep(0.3)
    F("after NoAI:0", i, "vehicle", a.state(0)["vehicle"], pos(a))
F([l[:100] for l in lines() if "Passengers" in l or "sound" in l or "Teleport" in l or "Position" in l][:15])
print(sv("data get entity @e[type=happy_ghast,limit=1]", show=False)[-600:])
