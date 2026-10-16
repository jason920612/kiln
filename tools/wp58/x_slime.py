from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 5.5 200 5.5", show=False)
time.sleep(1.5)
for size, n in ((0, 20), (1, 6)):
    clear_area()
    for i in range(n):
        sv(f"summon minecraft:slime 30.5 200 30.5 {{Size:{size},NoAI:1b}}", show=False, wait=0.1)
    time.sleep(1.0)
    sv("kill @e[type=slime]", show=False, wait=0.5)
    time.sleep(0.5)
    for _ in range(3):
        sv("kill @e[type=slime]", show=False, wait=0.4)
    time.sleep(1.0)
    out = sv("execute as @e[type=item] run data get entity @s Item.id", show=False, wait=1.0)
    ids = collections.Counter(re.findall(r'"minecraft:(\w+)"', out))
    cnt = sv("execute as @e[type=item] run data get entity @s Item.count", show=False, wait=1.0)
    total = sum(int(x) for x in re.findall(r"data: (\d+)|following entity data: (\d+)", cnt) and [m[0] or m[1] for m in re.findall(r"data: (\d+)|following entity data: (\d+)", cnt)])
    F("size", size, "x", n, "dropped", dict(ids), "count sum", total)
