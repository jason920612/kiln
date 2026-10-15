"""wp65: piglin bartering with real clicks, in the nether at the bastion: A wears a golden helmet (the piglins stay calm), right-clicks piglins
with gold ingots (B the same without helmet gets attacked-or-not); the loot thrown back is collected. Kiln (TARGET=K) vs vanilla (TARGET=V).
Piglins of the bastion's treasure are used when there are any near, else some are summoned on a platform."""
from p65_b2 import *

NETHER = "minecraft:the_nether"


def barter():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
        sv(f"give {nm} minecraft:gold_ingot 64", show=False)
        sv(f"give {nm} minecraft:diamond_sword 1", show=False)
    sv("give KilnTest minecraft:golden_helmet 1", show=False)
    sv("gamerule spawn_mobs false", show=False)
    # a platform in the nether with pens: 12 piglins that cannot zombify (immune), each stands still (NoAI would not barter: they need their brain)
    sv(f"execute in {NETHER} run forceload add 790 790 830 830", show=False)
    time.sleep(3)
    for _ in range(30):
        out = sv(f"execute in {NETHER} run setblock 800 99 800 minecraft:netherrack", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv(f"execute in {NETHER} run fill 794 98 794 816 99 806 minecraft:netherrack", show=False)
    sv(f"execute in {NETHER} run fill 794 100 794 816 106 806 minecraft:air", show=False)
    sv(f"execute in {NETHER} run fill 794 100 794 816 103 794 minecraft:nether_bricks", show=False)
    sv(f"execute in {NETHER} run fill 794 100 806 816 103 806 minecraft:nether_bricks", show=False)
    sv(f"execute in {NETHER} run fill 794 100 794 794 103 806 minecraft:nether_bricks", show=False)
    sv(f"execute in {NETHER} run fill 816 100 794 816 103 806 minecraft:nether_bricks", show=False)
    for nm in ("KilnTest", "Obs"):
        sv(f"execute in {NETHER} run tp {nm} 805.5 100 800.5", show=False)
    time.sleep(5)
    for i in range(12):
        sv(f"execute in {NETHER} run summon minecraft:piglin {797 + (i % 6) * 3}.5 100 {797 + (i // 6) * 6}.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,CanPickUpLoot:1b}}", show=False)
    time.sleep(4)
    a.select("minecraft:golden_helmet")
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("A wears", ent(b, eid_of(b))["head"] if eid_of(b) else None)
    a.select("minecraft:gold_ingot")
    done = 0
    t0 = time.time()
    sv("tick rate 60", show=False)
    # trade with each piglin: look at it, right-click
    for _ in range(14):
        ps = [e for e in a.state(30)["entities"] if e["t"] == "piglin"]
        if not ps:
            break
        st = a.state(0)
        p = min(ps, key=lambda e: (e["x"] - st["x"]) ** 2 + (e["z"] - st["z"]) ** 2)
        if interact_ent(a, p["id"], "right", tries=6, dy=0.9):
            done += 1
        time.sleep(0.3)
    F("trades started", done, "gold left", a.inv().get("minecraft:gold_ingot"))
    time.sleep(25)
    items = {}
    for _ in range(3):
        for e in a.state(30)["entities"]:
            if e["t"] == "item":
                items[e["id"]] = (e["item"], e["n"])
        time.sleep(2)
    # what lies around now and what A picked up
    for e in a.state(30)["entities"]:
        if e["t"] == "item":
            items[e["id"]] = (e["item"], e["n"])
    agg = {}
    for k, (it, n) in items.items():
        agg[it] = agg.get(it, 0) + n
    F("loot thrown (item entities seen)", {k.replace("minecraft:", ""): v for k, v in sorted(agg.items())})
    inv = a.inv()
    F("A inventory loot", {k.replace("minecraft:", ""): v for k, v in inv.items() if k not in ("minecraft:gold_ingot", "minecraft:diamond_sword", "minecraft:golden_helmet")})
    sv("tick rate 20", show=False)
    shot("barter")
    # B has no golden armour: the piglins attack B (resistance V keeps B alive): B's health stays, they target
    sv("gamerule spawn_mobs false", show=False)
    time.sleep(8)
    angry = sv(f"execute as @e[type=minecraft:piglin,x=794,y=98,z=794,dx=23,dy=10,dz=13] run data get entity @s Brain.memories", show=False, wait=1.5)
    F("piglins attacking B's side", len(re.findall(r"attack_target", angry)))


if __name__ == "__main__":
    barter()
