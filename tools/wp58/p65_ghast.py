"""wp65: a ghast fight with two real players on a nether platform: the ghast shoots, A hits the fireball back with the sword
("Return to Sender"), B shoots it down with a bow; soul sand slows the walker; Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p65_b2 import *

NETHER = "minecraft:the_nether"


def ghast():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"effect give {nm} minecraft:fire_resistance 3000 0 true", show=False)
        sv(f"give {nm} minecraft:diamond_sword 1", show=False)
        sv(f"give {nm} minecraft:bow 1", show=False)
        sv(f"give {nm} minecraft:arrow 64", show=False)
        sv(f"give {nm} minecraft:cooked_beef 32", show=False)
        sv(f"item replace entity {nm} armor.head with minecraft:diamond_helmet", show=False)
        sv(f"item replace entity {nm} armor.chest with minecraft:diamond_chestplate", show=False)
        sv(f"item replace entity {nm} armor.legs with minecraft:diamond_leggings", show=False)
        sv(f"item replace entity {nm} armor.feet with minecraft:diamond_boots", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("difficulty normal", show=False)
    sv("kill @e[type=!player]", show=False)
    sv(f"execute in {NETHER} run forceload add 780 780 830 840", show=False)
    time.sleep(3)
    for _ in range(30):
        out = sv(f"execute in {NETHER} run setblock 800 99 800 minecraft:netherrack", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv(f"execute in {NETHER} run fill 784 70 784 826 70 836 minecraft:netherrack", show=False)
    sv(f"execute in {NETHER} run fill 784 71 784 826 120 836 minecraft:air", show=False)
    sv(f"execute in {NETHER} run fill 796 98 796 806 98 806 minecraft:netherrack", show=False)
    sv(f"execute in {NETHER} run fill 796 99 796 806 110 806 minecraft:air", show=False)
    # soul sand strip next to the standing place
    sv(f"execute in {NETHER} run fill 796 98 802 806 98 814 minecraft:soul_sand", show=False)
    for nm in ("KilnTest", "Obs"):
        sv(f"execute in {NETHER} run tp {nm} 800.5 99 798.5", show=False)
    time.sleep(5)
    # soul sand: walking speed on it vs on netherrack (A walks 3 s each way)
    def dist_walked(c, seconds=2.0):
        p0 = pos(c)
        c.look(0, 0)
        c.key("w", "down"); time.sleep(seconds); c.key("w", "up")
        p1 = pos(c)
        return round(math.hypot(p1[0] - p0[0], p1[2] - p0[2]), 2)
    a.look(0, 0)
    sv(f"execute in {NETHER} run tp KilnTest 797.5 99 797.0", show=False)
    time.sleep(1.5)
    F("walk 1 s on netherrack:", dist_walked(a, 1.0))
    sv(f"execute in {NETHER} run tp KilnTest 797.5 99 802.6", show=False)
    time.sleep(1.5)
    F("walk 1 s on soul sand (+z):", dist_walked(a, 1.0))
    sv(f"execute in {NETHER} run tp KilnTest 800.5 99 798.5", show=False)
    time.sleep(1)
    # the ghast
    sv(f"execute in {NETHER} run summon minecraft:ghast 800.5 106 826.5", show=False)
    time.sleep(3)
    t0 = time.time()
    shots = 0
    seen_fb = set()
    reflected = 0
    ghast_hp = None
    a.select("minecraft:diamond_sword")
    b.select("minecraft:bow")
    while time.time() - t0 < 120:
        for c in (a, b):
            alive(c)
        gs = [e for e in a.state(60)["entities"] if e["t"] == "ghast"]
        fbs = [e for e in a.state(30)["entities"] if e["t"] in ("fireball", "small_fireball")]
        for f in fbs:
            if f["id"] not in seen_fb:
                seen_fb.add(f["id"])
                shots += 1
        if gs:
            ghast_hp = gs[0]["hp"]
            # B shoots the ghast
            b.shoot_ballistic(gs[0]["x"], gs[0]["y"] + 2.0, gs[0]["z"], charge=1.1)
        # A hits fireballs that come close
        near = [f for f in fbs if math.hypot(f["x"] - pos(a)[0], f["z"] - pos(a)[2]) < 5.0]
        if near:
            f = near[0]
            a.lookat(f["x"], f["y"], f["z"])
            time.sleep(0.1)
            a.mouse("left", "tap")
            reflected += 1
        time.sleep(0.6)
        if not gs and time.time() - t0 > 10:
            break
    F("fireballs seen", shots, "swings at fireballs", reflected, "ghast hp last", ghast_hp)
    F("ghast alive", len([e for e in a.state(80)["entities"] if e["t"] == "ghast"]), "A hp", a.state(0)["health"], "B hp", b.state(0)["health"])
    adv = chat()
    F("chat", [l for l in adv if "advancement" in l][:8])
    shot("ghast")


if __name__ == "__main__":
    ghast()
