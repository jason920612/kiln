from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 5.5 200 5.5", show=False)
time.sleep(1.5)
for typ in ("pig", "cow", "zombie", "slime", "skeleton", "chicken", "sheep"):
    for rep in range(2):
        clear_area()
        n = 12
        for i in range(n):
            sv(f"summon minecraft:{typ} 30.5 200 30.5 {{NoAI:1b{',Size:0' if typ=='slime' else ''}}}", show=False, wait=0.05)
        time.sleep(0.8)
        sv(f"kill @e[type={typ}]", show=False, wait=0.5)
        time.sleep(1.0)
        out = sv("execute as @e[type=item] run data get entity @s Item", show=False, wait=1.0)
        ids = collections.Counter(re.findall(r'id: "minecraft:(\w+)"', out))
        F(typ, "x", n, "killed with /kill; items:", dict(ids))
        if ids:
            break
