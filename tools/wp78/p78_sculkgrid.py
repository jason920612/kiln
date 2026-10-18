"""wp78: sculk blocks per chunk around the ancient city centre, both servers (par.sh), the chunks already loaded by p78_city_scout."""
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
if T == "V":
    sv("op KilnTest", show=False)
sv("gamemode creative KilnTest", show=False)
sv("execute in minecraft:overworld run tp KilnTest 816 -30 -576", show=False)
time.sleep(20)
cx0, cz0 = 816 // 16, -576 // 16
for nm in ("sculk", "sculk_vein", "sculk_sensor", "sculk_catalyst", "sculk_shrieker"):
    rows = []
    for dz in range(-3, 4):
        row = []
        for dx in range(-3, 4):
            x0, z0 = (cx0 + dx) * 16, (cz0 + dz) * 16
            r = a.findbox(nm, x0, -62, z0, x0 + 15, 10, z0 + 15, 3000)
            row.append(len(r))
        rows.append(row)
    print(nm)
    for row in rows:
        print("  ", " ".join(f"{n:4d}" for n in row))
