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
time.sleep(1.0)
def srv():
    out = sv("data get entity @e[type=happy_ghast,limit=1] Pos", show=False)
    m = re.findall(r"\[([-0-9.e]+)d, ([-0-9.e]+)d, ([-0-9.e]+)d\]", out)
    return tuple(round(float(v), 2) for v in m[-1]) if m else None
a.look(-90, 0)
trace = []
a.key("w", "down")
for i in range(10):
    time.sleep(0.5)
    gg = ghast_of(a, eid)
    trace.append((i, (round(gg["x"], 2), round(gg["y"], 2), round(gg["z"], 2)) if gg else None, srv(), pos(a)))
a.key("w", "up")
for t in trace:
    F("t", *t)
time.sleep(1.0)
F("after release client", [(round(e["x"],2), round(e["y"],2), round(e["z"],2)) for e in ents_of(a, "happy_ghast", 40)], "server", srv())
