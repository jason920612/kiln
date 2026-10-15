"""wp65 nether 2 (fortress): both players fight blazes at a fortress spawner and wither skeletons, loot a chest through the GUI."""
from p65_b2 import *

def n2():
    prep()
    for c, nm in ((a, "KilnTest"), (b, "Obs")):
        sv(f"clear {nm}", show=False)
        sv(f"give {nm} minecraft:diamond_sword 1", show=False)
        sv(f"give {nm} minecraft:bow 1", show=False)
        sv(f"give {nm} minecraft:arrow 64", show=False)
        sv(f"give {nm} minecraft:cooked_beef 32", show=False)
        sv(f"execute in minecraft:the_nether run tp {nm} 270.5 90 262.5", show=False)
    time.sleep(8)
    # the spawner's surroundings
    for c in (a,):
        sp = (270, 80, 260)
        spot = free_spot(c, 270, 80, 262, r=4, dy=2)
        F("free spot near the spawner", spot)
        for c2, nm in ((a, "KilnTest"), (b, "Obs")):
            sv(f"execute in minecraft:the_nether run tp {nm} {spot[1]+0.5} {spot[2]} {spot[3]+0.5}", show=False)
    time.sleep(3)
    shot("fortress_spawner")
    sv("tick rate 60", show=False)
    t0 = time.time(); kills = {"a": 0, "b": 0}
    while time.time() - t0 < 150:
        for key, c in (("a", a), ("b", b)):
            alive(c)
            bl = [e for e in c.state(30)["entities"] if e["t"] in ("blaze", "wither_skeleton")]
            if bl:
                c.select("minecraft:diamond_sword")
                if kill_nearest(c, bl[0]["t"], rng=30, timeout=10):
                    kills[key] += 1
        if kills["a"] + kills["b"] >= 8:
            break
        time.sleep(1)
    sv("tick rate 20", show=False)
    F("kills", kills, "A inv", a.inv(), "B inv", b.inv())
    es = [e["t"] for e in a.state(40)["entities"]]
    F("mobs seen near", {t: es.count(t) for t in set(es)})
    shot("fortress_fight")

if __name__ == "__main__":
    n2()
