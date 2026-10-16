from p68_small import *
sv("fill 18 200 18 24 203 24 minecraft:air", show=False)
sv("setblock 20 200 20 minecraft:lightning_rod[facing=up]", show=False)
sv("setblock 20 200 22 minecraft:redstone_lamp", show=False)
sv("setblock 20 200 21 minecraft:redstone_wire", show=False)
time.sleep(1)
for y in (200.0, 200.5, 201.0):
    sv(f"summon minecraft:lightning_bolt 20.5 {y} 20.5", show=False, wait=0.0)
    seen = []
    for _ in range(25):
        seen.append(("P" if "powered=true" in blk(a, 20, 200, 20) else ".") + ("L" if "lit=true" in blk(a, 20, 200, 22) else "."))
        time.sleep(0.05)
    print(y, " ".join(seen))
    time.sleep(1.5)
