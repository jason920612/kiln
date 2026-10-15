from p65_b2 import *
END = "minecraft:the_end"
cities = [(1280, 1328), (1968, -880), (-2176, 1008), (-1872, -1856), (2608, -1520), (1392, 2880), (1616, -2816), (-1536, -2768)]
prep()
for nm in ("KilnTest", "Obs"):
    sv(f"gamemode creative {nm}", show=False)
for cx, cz in cities:
    for nm in ("KilnTest", "Obs"):
        sv(f"execute in {END} run tp {nm} {cx}.5 100 {cz}.5", show=False)
    time.sleep(9)
    heads = a.findbox("dragon_", cx - 120, 30, cz - 120, cx + 120, 160, cz + 120, 3)
    print((cx, cz), "heads", heads, "shulkers", len([e for e in a.state(100)["entities"] if e["t"] == "shulker"]), flush=True)
    if heads:
        break
