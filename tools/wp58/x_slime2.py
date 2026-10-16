from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 20.5 200 20.5", show=False)
sv("gamemode survival KilnTest", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
time.sleep(1.5)
clear_area()
sv(f"clear {ME}", show=False)
give(ME, "minecraft:diamond_sword", 1)
a.select("minecraft:diamond_sword")
n = 14
for i in range(n):
    sv(f"summon minecraft:slime {20.5 + (i % 7) - 3} 200 {24.5 + i // 7} {{NoAI:1b,Size:0}}", show=False, wait=0.1)
time.sleep(1.0)
k = 0
for _ in range(n + 4):
    if kill_nearest(a, "slime", rng=20, timeout=10):
        k += 1
time.sleep(1.5)
a.collect(8, 4)
inv = sv(f"data get entity {ME} Inventory", show=False, wait=0.8)
balls = sum(int(x) for x in re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:slime_ball"', inv)) + sum(int(x) for x in re.findall(r'id: "minecraft:slime_ball", count: (\d+)', inv))
floor = sv("execute as @e[type=item] run data get entity @s Item.count", show=False, wait=1.0)
F("slimes killed with the sword:", k, "slime balls in inventory:", balls, "on the floor:", re.findall(r"data: (\d+)|entity data: (\d+)", floor))
F("expected about", n, "(uniform 0..2 each, one per kill on average)")
