from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 20.5 200 20.5", show=False)
sv("gamemode survival KilnTest", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
time.sleep(1.5)
def drops():
    out = sv("execute as @e[type=item] run data get entity @s Item", show=False, wait=1.0)
    floor = dict(collections.Counter(re.findall(r'id: "minecraft:(\w+)"', out)))
    inv = sv(f"data get entity {ME} Inventory", show=False, wait=0.8)
    got = collections.Counter()
    for n, name in re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:(\w+)"', inv):
        if name != "diamond_sword":
            got[name] += int(n)
    for name, n in re.findall(r'id: "minecraft:(\w+)", count: (\d+)', inv):
        if name != "diamond_sword":
            got[name] += int(n)
    return {"floor": floor, "picked up": dict(got)}
# 1. fire aspect sword on cows, plain sword on cows
for label, give_cmd in (("fire aspect sword", f"give {ME} minecraft:diamond_sword[enchantments={{\"minecraft:fire_aspect\":1}}] 1"), ("plain sword", f"give {ME} minecraft:diamond_sword 1")):
    clear_area()
    sv(f"clear {ME}", show=False)
    sv(give_cmd, show=False)
    time.sleep(0.5)
    a.select("minecraft:diamond_sword")
    for i in range(3):
        sv(f"summon minecraft:cow {20.5 + i - 1} 200 24.5 {{NoAI:1b}}", show=False)
    time.sleep(1.0)
    n = 0
    for _ in range(3):
        if kill_nearest(a, "cow", rng=20, timeout=12):
            n += 1
    time.sleep(1.0)
    F("cows killed with the", label, ":", n, drops())
# 2. sheep and slimes killed by sword
clear_area()
sv(f"clear {ME}", show=False)
give(ME, "minecraft:diamond_sword", 1)
a.select("minecraft:diamond_sword")
for i in range(3):
    sv(f"summon minecraft:sheep {20.5 + i - 1} 200 24.5 {{NoAI:1b}}", show=False)
    sv(f"summon minecraft:slime {20.5 + i - 1} 200 26.5 {{NoAI:1b,Size:0}}", show=False)
time.sleep(1.0)
k = 0
for t in ("sheep", "slime"):
    for _ in range(3):
        if kill_nearest(a, t, rng=20, timeout=12):
            k += 1
time.sleep(1.0)
F("sheep and slimes killed with a sword:", k, drops())
