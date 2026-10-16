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
F("riding", ok, a.state(0)["vehicle"], pos(a), "ghast", [(round(e["x"],2), round(e["y"],2), round(e["z"],2), e["bbh"]) for e in ents_of(a, "happy_ghast", 40)])
time.sleep(1.5)
rec_start(a)
a.key("shift", "down")
for i in range(12):
    time.sleep(0.2)
    gg = ghast_of(a, eid)
    F(i, "veh", a.state(0)["vehicle"], pos(a), "ghast y", gg and round(gg["y"], 2))
    if i == 2:
        a.key("shift", "up")
F([l[:90] for l in lines() if "Passengers" in l or "PlayerPosition" in l or "Teleport" in l or "MoveEntity" in l][:10])
print(sv("data get entity KilnTest Pos", show=False)[-80:])
