from p68_small import *
sv("kill @e[type=item]", show=False)
sv(f"tp {ME} 20.5 200 30.5", show=False)
sv(f"clear {ME}", show=False)
time.sleep(1.5)
res = []
for d in (1.0, 1.3, 1.5, 1.7, 2.0, 2.5):
    sv("kill @e[type=item]", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"summon minecraft:item {20.5+d} 200 30.5 {{Item:{{id:'minecraft:lead',count:1}},PickupDelay:0s,Motion:[0.0d,0.0d,0.0d]}}", show=False)
    time.sleep(1.2)
    res.append((d, a.inv().get("minecraft:lead", 0)))
print(res)
res = []
for d in (1.0, 1.5, 2.0):
    sv("kill @e[type=item]", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"summon minecraft:item {20.5} 200 {30.5+d} {{Item:{{id:'minecraft:lead',count:1}},PickupDelay:0s,Motion:[0.0d,0.0d,0.0d]}}", show=False)
    time.sleep(1.2)
    res.append((d, a.inv().get("minecraft:lead", 0)))
print("z", res)
res = []
for dy in (0.0, 1.0, 1.5, 2.0):
    sv("kill @e[type=item]", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"summon minecraft:item {20.5} {200+dy} {30.5+0.5} {{Item:{{id:'minecraft:lead',count:1}},PickupDelay:0s,NoGravity:1b,Motion:[0.0d,0.0d,0.0d]}}", show=False)
    time.sleep(1.2)
    res.append((dy, a.inv().get("minecraft:lead", 0)))
print("y", res)
sv("kill @e[type=item]", show=False)
