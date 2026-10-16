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
time.sleep(1.5)
def srv():
    out = sv("data get entity @e[type=happy_ghast,limit=1] Pos", show=False)
    m = re.findall(r"\[([-0-9.e]+)d, ([-0-9.e]+)d, ([-0-9.e]+)d\]", out)
    return tuple(round(float(v), 2) for v in m[-1]) if m else None
a.key("space", "down")
tr = []
for i in range(8):
    time.sleep(0.25)
    gg = ghast_of(a, eid)
    tr.append(("space", i, gg and round(gg["y"], 2), srv()))
a.key("space", "up")
for i in range(12):
    time.sleep(0.25)
    gg = ghast_of(a, eid)
    tr.append(("free", i, gg and round(gg["y"], 2), srv(), pos(a)))
for t in tr:
    F(*t)
