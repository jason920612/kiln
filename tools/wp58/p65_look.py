"""wp65: what B's client shows of A (armour, held items, animations): A does real things, B reads A's entity through `ent`.
Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p65_b2 import *


def look():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("fill -20 199 -20 20 199 20 minecraft:stone", show=False)
    sv("fill -20 200 -20 20 215 20 minecraft:air", show=False)
    sv("tp KilnTest 0.5 200 0.5", show=False)
    sv("tp Obs 0.5 200 4.5", show=False)
    sv("give KilnTest minecraft:diamond_helmet 1", show=False)
    sv("give KilnTest minecraft:iron_chestplate 1", show=False)
    sv("give KilnTest minecraft:golden_leggings 1", show=False)
    sv("give KilnTest minecraft:leather_boots[minecraft:dyed_color=16711680] 1", show=False)
    sv("give KilnTest minecraft:bow 1", show=False)
    sv("give KilnTest minecraft:arrow 20", show=False)
    sv("give KilnTest minecraft:cooked_beef 8", show=False)
    sv("give KilnTest minecraft:diamond_pickaxe 1", show=False)
    sv("give KilnTest minecraft:stone 64", show=False)
    sv("give KilnTest minecraft:shield 1", show=False)
    time.sleep(2)
    pa = eid_of(b)
    # B looks at A
    b.lookat(0.5, 201.2, 0.5)
    for it in ("diamond_helmet", "iron_chestplate", "golden_leggings", "leather_boots"):
        a.select("minecraft:" + it)
        a.mouse("right", "tap")
        time.sleep(0.6)
    e = ent(b, pa)
    F("B sees A wearing", e["head"], e["chest"], e["legs"], e["feet"])
    shot("armour_seen")
    # held items follow the hotbar
    for it in ("minecraft:bow", "minecraft:diamond_pickaxe", "minecraft:cooked_beef", "minecraft:stone"):
        a.select(it)
        time.sleep(0.5)
        F("A holds", it, "-> B sees main hand", ent(b, pa)["main"])
    # swing: A left-clicks the air
    a.select("minecraft:diamond_pickaxe")
    a.mouse("left", "tap")
    seen = []
    for _ in range(12):
        seen.append(round(ent(b, pa)["swingAnim"], 2))
        time.sleep(0.04)
    F("B sees A swing (swing progress samples)", seen)
    # eating: hold right click on food: B sees `using`
    sv("effect give KilnTest minecraft:hunger 3 5", show=False)
    time.sleep(0.5)
    a.select("minecraft:cooked_beef")
    a.mouse("right", "down")
    time.sleep(0.8)
    e = ent(b, pa)
    F("A eating: B sees using", e["using"], "main", e["main"])
    a.mouse("right", "up")
    time.sleep(0.5)
    # bow draw
    a.select("minecraft:bow")
    a.mouse("right", "down")
    time.sleep(0.8)
    e = ent(b, pa)
    F("A drawing a bow: B sees using", e["using"])
    a.mouse("right", "up")
    time.sleep(0.5)
    # sneaking / sprinting
    a.key("shift", "down")
    time.sleep(0.7)
    e = ent(b, pa)
    F("A sneaking: B sees crouch", e["crouch"], "pose", e["pose"])
    a.key("shift", "up")
    time.sleep(0.5)
    a.look(0, 0)
    a.key("w", "down"); a.key("ctrl", "down")
    time.sleep(0.7)
    e = ent(b, pa)
    F("A sprinting: B sees sprint", e["sprint"])
    a.key("w", "up"); a.key("ctrl", "up")
    # offhand shield
    a.select("minecraft:shield")
    a.key("f")
    time.sleep(0.6)
    F("A offhand", ent(b, pa)["off"])
    # sleep: B sees A in bed
    sv("setblock 6 200 0 minecraft:red_bed[facing=east,part=foot]", show=False)
    sv("setblock 7 200 0 minecraft:red_bed[facing=east,part=head]", show=False)
    sv("time set night", show=False)
    a.walk_to(5.5, 0.5, timeout=6, tol=0.8)
    a.lookat(6.5, 200.3, 0.5)
    time.sleep(0.5)
    a.mouse("right", "tap")
    time.sleep(2.5)
    e = ent(b, pa)
    F("A in bed: B sees sleeping", e["sleeping"], "pose", e["pose"], "A client sleeping", a.state(0)["sleeping"])
    shot("sleep_seen")
    sv("time set day", show=False)
    a.key("shift", "tap")
    time.sleep(1)
    F("after waking: B sees pose", ent(b, pa)["pose"])
    # on fire
    sv("effect clear KilnTest minecraft:resistance", show=False)
    sv("data merge entity @e[type=player,name=KilnTest,limit=1] {Fire:100}", show=False)
    time.sleep(0.6)
    F("A burning: B sees onFire", ent(b, pa)["onFire"])
    sv("effect give KilnTest minecraft:fire_resistance 100 0 true", show=False)
    sv("effect give KilnTest minecraft:invisibility 100 0 true", show=False)
    time.sleep(0.6)
    e = ent(b, pa)
    F("A invisible: B sees invisible", e["invisible"], "main", e["main"], "chest", e["chest"])


if __name__ == "__main__":
    look()
