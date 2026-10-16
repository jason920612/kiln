from p68_ghast import *
setup_two()
F("pos", pos(a), blk(a, 20, 199, 20), blk(a, 20, 200, 20))
kill_all_mobs()
sv("fill 18 200 18 24 204 24 minecraft:air", show=False)
give("KilnTest", "minecraft:dried_ghast", 1); give("KilnTest", "minecraft:water_bucket", 1)
sv("tp KilnTest 20.5 200 20.5", show=False); time.sleep(1.5)
a.select("minecraft:dried_ghast"); a.lookat(21.5, 199.99, 23.5); time.sleep(0.8)
a.mouse("right", "tap"); time.sleep(0.8)
hits = a.find("dried_ghast", 8, 4); F("ghast", hits)
x, y, z = hits[0]
for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1)):
    sv(f"setblock {x+dx} {y} {z+dz} minecraft:stone", show=False)
    if (dx, dz) != (0, -1):
        sv(f"setblock {x+dx} {y+1} {z+dz} minecraft:stone", show=False)
sv(f"setblock {x} {y+1} {z-1} minecraft:stone_slab[type=bottom]", show=False)
time.sleep(0.8)
a.select("minecraft:water_bucket")
a.lookat(x + 0.5, y + 0.9, z + 0.5); time.sleep(0.8)
F("aim", a.state(0).get("hit"), pos(a), a.state(0)["selected"], a.inv())
a.mouse("right", "tap"); time.sleep(1.0)
F("after", blk(a, x, y, z), blk(a, x, y+1, z), a.inv())
