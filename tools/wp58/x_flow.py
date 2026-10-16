from p68_ghast import *
kill_all_mobs()
sv("tp KilnTest 26.5 200 22.5", show=False)
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
a.look(-90, -10)
a.key("w", "down"); time.sleep(4.0); a.key("w", "up")
a.key("space", "down"); time.sleep(2.0); a.key("space", "up")
def srv():
    out = sv("data get entity @e[type=happy_ghast,limit=1] Pos", show=False)
    m = re.findall(r"\[([-0-9.e]+)d, ([-0-9.e]+)d, ([-0-9.e]+)d\]", out)
    return tuple(round(float(v), 2) for v in m[-1]) if m else None
def full(c):
    en = json.loads(c.cmd(f"ent {eid}"))
    return repr(en["y"])
def srvy():
    out = sv("data get entity @e[type=happy_ghast,limit=1] Pos", show=False)
    m = re.findall(r"\[([-0-9.eE]+)d, ([-0-9.eE]+)d, ([-0-9.eE]+)d\]", out)
    return m[-1][1] if m else None
time.sleep(1.5)
F("ghast y client", full(a), "server", srvy(), "player", repr(a.state(0)["y"]))
rec_start(a)
a.key("shift", "down"); time.sleep(0.3); a.key("shift", "up")
for i in range(8):
    time.sleep(0.2)
    F("after shift", i, "player y", repr(a.state(0)["y"]), "ghast client", full(a), "server", srvy(), "veh", a.state(0)["vehicle"])
