"""wp65: PvP to the death with real clicks: B kills A with an axe; messages, drops, picking up, respawn; then an ender pearl, TNT and a bow between the two.
Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p65_b2 import *


def pvp2():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"attribute {nm} minecraft:max_health base set 20", show=False)
    sv("gamerule keep_inventory false", show=False)
    sv("fill -20 199 -20 20 199 20 minecraft:stone", show=False)
    sv("fill -20 200 -20 20 215 20 minecraft:air", show=False)
    sv("kill @e[type=item]", show=False)
    sv("give KilnTest minecraft:cobblestone 12", show=False)
    sv("give KilnTest minecraft:diamond 3", show=False)
    sv("give Obs minecraft:diamond_axe 1", show=False)
    sv("give Obs minecraft:bow 1", show=False)
    sv("give Obs minecraft:arrow 32", show=False)
    sv("tp KilnTest 0.5 200 0.5", show=False)
    sv("tp Obs 0.5 200 2.5", show=False)
    time.sleep(3)
    chat()
    chat_b = None
    b.select("minecraft:diamond_axe")
    time.sleep(1)
    t0 = time.time()
    hits = 0
    while time.time() - t0 < 40:
        if a.screen() == "DeathScreen":
            break
        sa = a.state(0)
        sb = b.state(0)
        if math.hypot(sa["x"] - sb["x"], sa["z"] - sb["z"]) > 2.4:
            b.walk_to(sa["x"], sa["z"], timeout=2, tol=1.5)
        b.lookat(sa["x"], sa["y"] + 1.0, sa["z"])
        time.sleep(0.9)
        b.mouse("left", "tap")
        hits += 1
        time.sleep(0.4)
    F("A died after", hits, "swings:", a.screen())
    time.sleep(1.5)
    log = chat()
    F("chat lines about the death", [l for l in log if "slain" in l or "killed" in l or "Obs" in l and "has made" not in l][:4])
    items = [(e["item"], e["n"]) for e in b.state(12)["entities"] if e["t"] == "item"]
    F("dropped items B sees", sorted(items))
    # B picks them up
    b.collect(8, 12)
    F("B inventory after picking up", {k.replace("minecraft:", ""): v for k, v in b.inv().items()})
    alive(a)
    F("A respawned at", pos(a), "hp", a.state(0)["health"], "inv", a.inv())
    # an ender pearl: B throws at the floor ahead
    sv("give Obs minecraft:ender_pearl 4", show=False)
    sv("tp Obs 0.5 200 0.5", show=False)
    time.sleep(1.5)
    b.select("minecraft:ender_pearl")
    b.look(0, 20)
    p0 = pos(b)
    h0 = b.state(0)["health"]
    b.mouse("right", "tap")
    time.sleep(3.0)
    p1 = pos(b)
    F("B threw a pearl: moved", round(math.hypot(p1[0] - p0[0], p1[2] - p0[2]), 1), "blocks, hp", h0, "->", b.state(0)["health"])
    # an arrow from B at A
    sv("tp KilnTest 0.5 200 14.5", show=False)
    sv("tp Obs 0.5 200 0.5", show=False)
    time.sleep(2)
    b.select("minecraft:bow")
    ha = a.state(0)["health"]
    sa = a.state(0)
    b.shoot_ballistic(sa["x"], sa["y"] + 1.0, sa["z"], charge=1.3)
    time.sleep(2.5)
    F("arrow from B: A hp", ha, "->", a.state(0)["health"])
    shot("pvp2")
    sv("kill @e[type=item]", show=False)


if __name__ == "__main__":
    pvp2()
