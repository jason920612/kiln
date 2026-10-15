"""wp66 item 3: sugar cane growth as a random tick counter. Columns of cane (grass, water on the odd x) in NCX x NCZ chunks; the total random
ticks a column got = 16 * (height - 1) + age of its top block (below 32).  usage: TARGET=K|V rt_cane.py SPEED TICKS NCX NCZ [OUT.pkl]"""
import sys, pickle
from p66rt import *
import anvil

speed, ticks, NCX, NCZ = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
OUT = sys.argv[5] if len(sys.argv) > 5 else None
BX, BZ = 1024, 1024
WORLD = DIR / "world"
YB = -60   # the base block of the cane (the flat world's grass is at -61)


def save():
    t0 = time.time() - 0.2
    run(["save-all flush"], timeout=600)
    files = {WORLD / "dimensions" / "minecraft" / "overworld" / "region" / f"r.{(BX//16+i)>>5}.{(BZ//16+j)>>5}.mca" for i in range(NCX) for j in range(NCZ)}
    for _ in range(600):
        if all(f.exists() and f.stat().st_mtime >= t0 for f in files):
            time.sleep(0.15)
            return
        time.sleep(0.1)
    raise RuntimeError("region files not written")


def build():
    cmds = []
    for i in range(NCX):
        for j in range(NCZ):
            x0, z0 = BX + 16 * i, BZ + 16 * j
            cmds.append(f"fill {x0} {YB} {z0} {x0+15} {YB+3} {z0+15} minecraft:air")
            cmds.append(f"fill {x0} {YB-1} {z0} {x0+15} {YB-1} {z0+15} minecraft:grass_block")
            for dx in range(1, 16, 2):
                cmds.append(f"fill {x0+dx} {YB-1} {z0} {x0+dx} {YB-1} {z0+15} minecraft:water")
            for dx in range(0, 16, 2):
                cmds.append(f"fill {x0+dx} {YB} {z0} {x0+dx} {YB} {z0+15} minecraft:sugar_cane")
    return sum(counts(cmds, timeout=3600))


def read():
    """total random ticks per column: list of (x, z, h, age_top, total)"""
    out = []
    for i in range(NCX):
        for j in range(NCZ):
            ch = anvil.chunk_nbt(WORLD, BX // 16 + i, BZ // 16 + j)
            pal, idx = anvil.section_states(ch, (YB) // 16)
            for x in range(0, 16, 2):
                for z in range(16):
                    h, top = 0, 0
                    for dy in range(0, 5):
                        y = (YB + dy) & 15
                        n, p = pal[idx[y * 256 + z * 16 + x]]
                        if n == "minecraft:sugar_cane":
                            h += 1
                            top = int(p.get("age", 0))
                        else:
                            break
                    out.append((BX + 16 * i + x, BZ + 16 * j + z, h, top, 16 * (h - 1) + top))
    return out


run([f"forceload add {BX} {BZ} {BX+16*NCX-1} {BZ+16*NCZ-1}"])
assert ready(BX, BZ) and ready(BX + 16 * NCX - 1, BZ + 16 * NCZ - 1)
run(["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "time set day", "weather clear 1000000"])
t0 = time.time()
print("built", build(), f"changes in {time.time()-t0:.0f}s", flush=True)
save()
r0 = read()
assert all(t[4] == 0 for t in r0), r0[:3]
run([f"gamerule random_tick_speed {speed}"])
t0 = time.time()
n = advance(ticks)
run(["gamerule random_tick_speed 0"])
print(f"advanced {n} ticks in {time.time()-t0:.0f}s", flush=True)
save()
r = read()
tot = [t[4] for t in r]
cens = sum(1 for t in r if t[2] >= 3)
m = sum(tot) / len(tot)
exp = ticks * speed / 4096
var = sum((t - m) ** 2 for t in tot) / len(tot)
print(f"TARGET={T} speed={speed} ticks={ticks} columns={len(r)} mean hits {m:.3f} expected {exp:.3f} ratio {m/exp:.4f}  sd {var**0.5:.2f} (poisson {exp**0.5:.2f})  height-3 columns {cens}")
if OUT:
    pickle.dump({"speed": speed, "ticks": ticks, "target": T, "cols": r}, open(OUT, "wb"))
