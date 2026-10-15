"""wp65: an end city with a ship: find the ship (dragon head), the item frame with the elytra, shulkers (levitation), the chests' loot;
A takes the elytra from the frame with real clicks and wears it. Kiln (TARGET=K) vs vanilla (TARGET=V): both servers have the same city."""
from p65_b2 import *

END = "minecraft:the_end"


def city(cx, cz):
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 3 true", show=False)
        sv(f"give {nm} minecraft:diamond_sword 1", show=False)
        sv(f"give {nm} minecraft:cooked_beef 16", show=False)
        sv(f"effect give {nm} minecraft:slow_falling 20 0 true", show=False)
        sv(f"execute in {END} run tp {nm} {cx}.5 110 {cz}.5", show=False)
    time.sleep(12)
    F("A at", pos(a), dim(a))
    heads = a.findbox("dragon_", cx - 130, 30, cz - 130, cx + 130, 160, cz + 130, 5)
    F("dragon heads (ships)", heads)
    shulk = len([e for e in a.state(100)["entities"] if e["t"] == "shulker"])
    F("shulkers within 100", shulk)
    if not heads:
        return
    hx, hy, hz = heads[0]
    sv(f"execute in {END} run tp KilnTest {hx} {hy + 12} {hz}", show=False)
    sv(f"execute in {END} run tp Obs {hx} {hy + 12} {hz}", show=False)
    time.sleep(8)
    frames = sv(f"execute in {END} as @e[type=minecraft:item_frame,x={hx - 20},y={hy - 20},z={hz - 20},dx=40,dy=40,dz=40] run data get entity @s Item", show=False, wait=1.5)
    F("item frames near the ship", re.findall(r'id: "minecraft:([a-z_]+)"', frames))
    chests = a.findbox("chest", hx - 20, hy - 20, hz - 20, hx + 20, hy + 20, hz + 20, 8)
    F("chests near the ship", chests)
    seen = {}
    for pos_ in chests:
        spot = free_spot(a, pos_[0], pos_[1], pos_[2], r=3, dy=2)
        if not spot:
            continue
        sv(f"execute in {END} run tp KilnTest {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}", show=False)
        time.sleep(3)
        a.close()
        a.use_on(*pos_)
        time.sleep(1.2)
        if a.screen() and "Container" in a.screen():
            its = [(s["id"].replace("minecraft:", ""), s["n"]) for s in a.state(0).get("slots", []) if s["i"] < 27 and s["n"] > 0]
            seen[str(tuple(pos_))] = its
        a.close()
    F("chest loot", seen)
    # the elytra frame: hit it (real)
    fr = [e for e in a.state(30)["entities"] if e["t"] == "item_frame"]
    F("frames the client sees", len(fr))
    got = False
    for f in fr:
        sv(f"execute in {END} run tp KilnTest {f['x'] + 1.5} {f['y'] - 0.5} {f['z']}", show=False)
        time.sleep(2)
        a.lookat(f["x"], f["y"], f["z"])
        time.sleep(0.6)
        a.mouse("left", "tap"); time.sleep(0.6)
        a.mouse("left", "tap"); time.sleep(1.2)
        if "minecraft:elytra" in str(a.inv()) or any("elytra" in k for k in a.inv()):
            got = True
            break
        # items dropped around
        a.collect(5, 6)
        if any("elytra" in k for k in a.inv()):
            got = True
            break
    F("A got an elytra from a frame", got, a.inv())
    if got:
        a.select("minecraft:elytra"); a.mouse("right", "tap"); time.sleep(0.8)
        F("A wears", ent(b, eid_of(b))["chest"] if eid_of(b) else None)
    # shulkers near: a bullet gives levitation
    sh = [e for e in a.state(60)["entities"] if e["t"] == "shulker"]
    F("shulkers near the ship", len(sh))
    sv("effect clear KilnTest minecraft:resistance", show=False)
    t0 = time.time()
    lev = False
    if sh:
        s = min(sh, key=lambda e: (e["x"] - pos(a)[0]) ** 2 + (e["z"] - pos(a)[2]) ** 2)
        sv(f"execute in {END} run tp KilnTest {s['x'] + 2} {s['y']} {s['z']}", show=False)
        while time.time() - t0 < 40:
            if any("levitation" in e["id"] for e in a.state(0)["effects"]):
                lev = True
                break
            time.sleep(1)
    F("levitation from a shulker bullet", lev)
    F("mspt", mspt())
    shot("end_city")


if __name__ == "__main__":
    city(int(sys.argv[1]) if len(sys.argv) > 1 else -576, int(sys.argv[2]) if len(sys.argv) > 2 else 992)
