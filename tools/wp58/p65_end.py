"""wp65: the End after the dragon: kill it, look at the exit portal and the first gateway, then both players use the gateway
(walk in) and arrive on the outer island; the same on Kiln (TARGET=K) and vanilla (TARGET=V)."""
from p65_b2 import *

def end1():
    prep()
    E = "minecraft:the_end"
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode creative {nm}", show=False)
        sv(f"execute in {E} run tp {nm} 0.5 70 40.5", show=False)
    time.sleep(10)
    F("end dim", dim(a), pos(a))
    es = [e["t"] for e in a.state(120)["entities"]]
    F("entities seen", {t: es.count(t) for t in set(es)})
    # the dragon and the crystals
    sv("execute in %s run kill @e[type=minecraft:ender_dragon]" % E, show=False)
    t0 = time.time()
    while time.time() - t0 < 60:
        time.sleep(2)
        es = [e["t"] for e in a.state(160)["entities"]]
        if "end_crystal" in es and False: pass
        out = a.find("end_portal", 60, 5)
        if out: break
    F("exit portal found after", round(time.time() - t0), "s", out[:2] if out else None)
    time.sleep(5)
    gw = a.findbox("end_gateway", -120, 60, -120, 120, 100, 120, 5)
    F("gateways", gw)
    F("egg", a.findbox("dragon_egg", -3, 60, -3, 3, 100, 3, 3))
    es = [e for e in a.state(160)["entities"]]
    F("xp orbs", len([e for e in es if e["t"] == "experience_orb"]))
    shot("end_after_dragon")
    if gw:
        g = gw[0]
        r = sv(f"execute in {E} run data get block {g[0]} {g[1]} {g[2]}", wait=1.0, show=False)
        F("gateway data", re.sub(r"^.*?\{", "{", r.strip())[:200])

if __name__ == "__main__" and "end1" in sys.argv:
    end1()

def end2():
    """both players step into the first gateway (after end1)"""
    E = "minecraft:the_end"
    for nm in ("KilnTest", "Obs"):
        sv(f"gamemode survival {nm}", show=False)
        sv(f"effect give {nm} minecraft:slow_falling 600 0 true", show=False)
        sv(f"effect give {nm} minecraft:resistance 600 4 true", show=False)
        sv(f"execute in {E} run tp {nm} 94.5 76 0.5", show=False)
    time.sleep(3)
    t0 = time.time()
    for nm in ("KilnTest", "Obs"):
        sv(f"execute in {E} run tp {nm} 96.5 75 0.5", show=False)
    for _ in range(40):
        time.sleep(1)
        if abs(pos(a)[0] - 96.5) > 20: break
    F("A after the gateway at", pos(a), "after", round(time.time() - t0, 1), "s", "B", pos(b))
    time.sleep(8)
    F("A", pos(a), "dim", dim(a), "B", pos(b))
    F("island blocks under A", a.block(math.floor(pos(a)[0]), math.floor(pos(a)[1]) - 1, math.floor(pos(a)[2])))
    gw = a.find("end_gateway", 30, 10)
    F("gateways near the exit", gw)
    shot("end_outer_island")
    F("mspt", mspt())
    F("chunks", chunks())

if __name__ == "__main__" and "end2" in sys.argv:
    end2()
