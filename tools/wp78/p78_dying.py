"""wp78: the dragon's death sequence in isolation (two creative clients on one server, TARGET=K|V): the dragon is put into the DYING phase with 1 health
(setup), the clients only watch and pick up the experience. Logs every 0.5 s: where the dragon is, its phase, what appears (orbs, portal, egg, gateway, items)."""
import collections, sys
from p72 import *
import p61
A, B = C(25999), C(26000)
ME = ["KilnTest", "Obs"]

def say(*x): print(*x, flush=True)
def E(cmd, wait=0.4): return sv("execute in minecraft:the_end run " + cmd, show=False, wait=wait)

if T == "V":
    sv("op KilnTest", show=False); sv("op Obs", show=False)
alive_auto(A); alive_b(B)
for nm in ME:
    sv(f"gamemode creative {nm}", show=False)
    sv(f"effect give {nm} minecraft:slow_falling 99999 0 true", show=False)
    E(f"tp {nm} 12 75 12")
time.sleep(10)
xp0 = (A.state(0).get("xp") or 0) + (B.state(0).get("xp") or 0)
have = [e for e in A.state(200)["entities"] if e["t"] == "ender_dragon"]
say("dragon present:", bool(have), "bar", A.cmd("bossbars")[:90])
E("data merge entity @e[type=ender_dragon,limit=1] {Health:1.0f,DragonPhase:9}")
t0 = time.time()
last = None
gone = None
for s in range(400):
    es = A.state(200)["entities"]
    dr = [e for e in es if e["t"] == "ender_dragon"]
    cnt = dict(collections.Counter(e["t"] for e in es if e["t"] in ("experience_orb", "end_crystal", "area_effect_cloud", "item")))
    portal = len(A.findbox("end_portal", -6, 55, -6, 6, 75, 6, 50))
    egg = len(A.findbox("dragon_egg", -6, 55, -6, 6, 80, 6, 5))
    gw = len(A.findbox("end_gateway", -120, 50, -120, 120, 120, 120, 5))
    bar = A.cmd("bossbars")[:40]
    row = ("dragon", (round(dr[0]["x"]), round(dr[0]["y"]), round(dr[0]["z"])) if dr else None, cnt, "portal", portal, "egg", egg, "gateway", gw, "bar", bar)
    if not dr and gone is None:
        gone = time.time() - t0
    if row != last:
        say(round(time.time() - t0, 1), *row)
    last = row
    if gone is not None and time.time() - t0 > gone + 15 and portal:
        break
    time.sleep(0.4)
for _ in range(8):
    orbs = [e for e in A.state(200)["entities"] if e["t"] == "experience_orb"]
    if not orbs:
        break
    for nm in ME:
        E(f"tp {nm} {orbs[0]['x']} {orbs[0]['y']} {orbs[0]['z']}")
    time.sleep(3)
xp1 = (A.state(0).get("xp") or 0) + (B.state(0).get("xp") or 0)
say("experience gained by the two players:", xp1 - xp0)
shot("end_dying_" + T)
