"""wp65 batch 2: nether portals with two real clients (both stand in one portal), Kiln (TARGET=K) vs vanilla (TARGET=V).
usage: p65_b2.py link|search ..."""
from p65 import *
import sys

a, b = clients()
which = sys.argv[1:] or ["link"]
FACTS = []


def F(*x):
    s = " ".join(str(i) for i in x)
    FACTS.append(s)
    print("F:", s, flush=True)


def dim(c):
    return c.state(0)["dimension"].split(":")[-1].rstrip("]")


def pos(c):
    s = c.state(0)
    return round(s["x"], 1), round(s["y"], 1), round(s["z"], 1)


def tpd(c, nm, x, y, z):
    """tp in the dimension the player is in (the console's own dimension is the overworld)."""
    sv(f"execute in minecraft:{dim(c)} run tp {nm} {x} {y} {z}", show=False)


def frame(x, y, z, axis="x", tag=""):
    """A 4x5 obsidian frame whose lower-left interior block is (x, y, z); `axis` is the portal's axis."""
    if axis == "x":
        sv(f"fill {x-1} {y-1} {z} {x+2} {y+3} {z} minecraft:obsidian", show=False)
        sv(f"fill {x} {y} {z} {x+1} {y+2} {z} minecraft:air", show=False)
    else:
        sv(f"fill {x} {y-1} {z-1} {x} {y+3} {z+2} minecraft:obsidian", show=False)
        sv(f"fill {x} {y} {z} {x} {y+2} {z+1} minecraft:air", show=False)


def light(x, y, z):
    sv(f"setblock {x} {y} {z} minecraft:fire", show=False)


def prep(mode="survival"):
    if T == "V":
        sv("op KilnTest", show=False)
        sv("op Obs", show=False)
    alive(a)
    alive(b)
    sv("gamerule spawn_mobs false", show=False)
    sv("difficulty normal", show=False)
    sv("time set day", show=False)
    sv("weather clear", show=False)
    for p in ("KilnTest", "Obs"):
        sv(f"gamemode {mode} {p}", show=False)
        sv(f"effect give {p} minecraft:fire_resistance 3000 0 true", show=False)
        sv(f"effect give {p} minecraft:resistance 3000 4 true", show=False)


def portals(c, r=40):
    """nether_portal blocks the client sees around the player, grouped to (min x, y, z) of each connected portal."""
    ps = [tuple(p) for p in c.find("nether_portal", r, 400)]
    seen, out = set(), []
    for p in sorted(ps):
        if p in seen:
            continue
        stack, comp = [p], []
        seen.add(p)
        while stack:
            q = stack.pop()
            comp.append(q)
            for d in ((1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)):
                n = (q[0] + d[0], q[1] + d[1], q[2] + d[2])
                if n in ps and n not in seen:
                    seen.add(n)
                    stack.append(n)
        out.append((min(x for x, _, _ in comp), min(y for _, y, _ in comp), min(z for _, _, z in comp), len(comp)))
    return out


def wait_dim(c, want, timeout=25):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if dim(c) == want and c.state(0)["inGame"] and c.screen() is None:
            return time.time() - t0
        time.sleep(0.4)
    return None


def enter_both(x, y, z, want, axis="x"):
    """Both players walk into the portal interior at (x, y, z) and wait inside until they are in dimension `want`."""
    for c, off in ((a, 0.3), (b, 0.7)):
        pass
    t0 = time.time()
    for c, nm in ((a, "KilnTest"), (b, "Obs")):
        sv(f"tp {nm} {x + (0.5 if axis == 'x' else 0.5)} {y} {z + (0.5 if axis == 'z' else 0.5)}", show=False)
    time.sleep(0.3)
    ta = wait_dim(a, want, 30)
    tb = wait_dim(b, want, 30)
    return ta, tb, round(time.time() - t0, 1)


def link():
    prep()
    # P1 on a sky platform in the overworld, far from the world spawn (808, 200, 808) -> nether (101, ?, 101)
    sv("execute in minecraft:overworld run forceload add 800 800 830 830", show=False)
    time.sleep(2)
    for i in range(10):
        out = sv("setblock 805 199 805 minecraft:stone", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv("fill 798 199 798 835 199 835 minecraft:stone", show=False)
    sv("fill 798 200 798 835 215 835 minecraft:air", show=False)
    frame(808, 200, 808, "x")
    light(808, 200, 808)
    sv("tp KilnTest 805 200 805", show=False)
    sv("tp Obs 806 200 805", show=False)
    time.sleep(2)
    F("P1 lit:", portals(a, 12))
    # both walk into the portal (A and B stand inside it): the real teleport delay is 80 ticks
    sv("tp KilnTest 808.5 200 808.5", show=False)
    sv("tp Obs 809.5 200 808.5", show=False)
    t0 = time.time()
    ta = wait_dim(a, "the_nether", 40)
    tb = wait_dim(b, "the_nether", 40)
    F("both in the nether after", ta and round(ta, 1), tb and round(tb, 1))
    time.sleep(3)
    F("A in nether at", pos(a), "B at", pos(b))
    F("nether portals near A:", portals(a, 30))
    shot("nether_arrive")
    # --- both go back through the nether portal: they arrive at P1 (the closest overworld portal), no new portal ---
    def hop(nm_list, x, y, z, want, dx=0.5, dz=0.5):
        # step out of the portal we arrived in (standing in it keeps the cooldown full), let the 300 ticks run out
        for c, nm in ((a, "KilnTest"), (b, "Obs")):
            st = c.state(0)
            tpd(c, nm, st["x"] + 4, st["y"] + 0.2, st["z"] + 4)
        time.sleep(17)
        for nm in nm_list:
            tpd(a if nm == "KilnTest" else b, nm, x + dx, y, z + dz)
        ta = wait_dim(a, want, 40)
        tb = wait_dim(b, want, 40)
        time.sleep(2.5)
        return ta and round(ta, 1), tb and round(tb, 1)
    tpd(a, "KilnTest", 99, 63, 105)
    tpd(b, "Obs", 99, 63, 105)
    time.sleep(2.5)
    npc = portals(a, 40)
    px = [(p[0], p[1], p[2]) for p in npc][0]
    r = hop(["KilnTest", "Obs"], px[0], px[1], px[2], "overworld")
    F("back to the overworld after", r, "A", pos(a), "B", pos(b), "overworld portals", portals(a, 60))
    # --- P2 at x=888 (nether x 111, 14 from the existing nether portal, inside the 16 search radius) ---
    for x in (888, 968):
        sv(f"execute in minecraft:overworld run forceload add {x-10} 800 {x+10} 830", show=False)
    time.sleep(3)
    for x in (888, 968):
        for i in range(10):
            out = sv(f"setblock {x} 199 805 minecraft:stone", show=False, wait=0.8)
            if "unloaded" not in out and "not loaded" not in out:
                break
            time.sleep(1.5)
        sv(f"fill {x-10} 199 798 {x+10} 199 835 minecraft:stone", show=False)
        sv(f"fill {x-10} 200 798 {x+10} 215 835 minecraft:air", show=False)
        frame(x, 200, 808, "x")
        light(x, 200, 808)
    time.sleep(1.5)
    r = hop(["KilnTest", "Obs"], 888, 200, 808, "the_nether")
    F("P2 (x 888) -> nether after", r, "A", pos(a), "nether portals near", portals(a, 60))
    # back from there: the closest overworld portal to (nether x*8) wins
    npc = portals(a, 40)
    F("nether portal count near the arrival", len(npc))
    px = (npc[0][0], npc[0][1], npc[0][2])
    r = hop(["KilnTest", "Obs"], px[0], px[1], px[2], "overworld")
    F("back again: A", pos(a), "B", pos(b))
    # --- P3 at x=968 (nether x 121, 24 from the nether portal): a second nether portal is created ---
    r = hop(["KilnTest", "Obs"], 968, 200, 808, "the_nether")
    F("P3 (x 968) -> nether after", r, "A", pos(a), "nether portals near", portals(a, 60))
    npc = portals(a, 40)
    F("nether portal count near the arrival", len(npc))
    px = [(p[0], p[1], p[2]) for p in npc if abs(p[0] - a.state(0)["x"]) < 4][0]
    r = hop(["KilnTest", "Obs"], px[0], px[1], px[2], "overworld")
    F("back from the second nether portal: A", pos(a), "B", pos(b))
    # one more from the first nether portal: it goes back to P1, the closest one in overworld distance
    r = hop(["KilnTest", "Obs"], 808, 200, 808, "the_nether")
    F("P1 again -> nether: A", pos(a))
    F("done; mspt", mspt())


def search():
    pass


NAMES = {"link": link, "search": search}
if __name__ == "__main__":
    for w in which:
        print("=====", w)
        NAMES[w]()
    json.dump(FACTS, open(OUT / f"b2_{'_'.join(which)}_{T}.json", "w"), indent=1)
