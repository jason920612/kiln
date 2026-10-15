"""wp66 item 3: random tick rates of wheat, kelp and saplings, counted from the world files (no block changes after the build).
A block of NCX x NCZ chunks, SPEED random_tick_speed, TICKS ticks (`tick sprint`), then `save-all flush` and the plants are read:
  wheat   - age histogram (farmland with water rows, all light)           mean age
  kelp    - height of each column (sand, 27 deep water, kelp on every 2nd column)   mean growth
  sapling - fraction at stage 1 (a glass roof two above keeps the tree from growing, so stage 1 is final)   fraction -> mean number of random ticks
usage: TARGET=K|V rt_crops.py PLANT SPEED TICKS NCX NCZ OUT.json"""
import sys, json, math, time
from p66rt import *
import anvil

plant, speed, ticks, NCX, NCZ, OUT = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), sys.argv[6]
BX, BZ = int(os.environ.get("CX", "1024")), int(os.environ.get("CZ", "1024"))
WORLD = DIR / "world"
G = -61     # the ground (flat world: grass at -61)


def save():
    t0 = time.time() - 0.2
    run(["save-all flush"], timeout=900)
    files = {WORLD / "dimensions" / "minecraft" / "overworld" / "region" / f"r.{(BX//16+i)>>5}.{(BZ//16+j)>>5}.mca" for i in range(NCX) for j in range(NCZ)}
    for _ in range(100):
        if all(f.exists() and f.stat().st_mtime >= t0 for f in files):
            break
        time.sleep(0.1)
    time.sleep(0.2)


def build():
    cmds = []
    for i in range(NCX):
        for j in range(NCZ):
            x0, z0 = BX + 16 * i, BZ + 16 * j
            x1, z1 = x0 + 15, z0 + 15
            cmds.append(f"fill {x0} {G-1} {z0} {x1} {G-1} {z1} minecraft:dirt")
            if plant == "wheat":
                cmds += [f"fill {x0} {G} {z0} {x1} {G} {z1} minecraft:farmland[moisture=7]",
                         f"fill {x0} {G+1} {z0} {x1} {G+3} {z1} minecraft:air",
                         f"fill {x0} {G+1} {z0} {x1} {G+1} {z1} minecraft:wheat[age=0]"]
                for wx in (x0 + 4, x0 + 13):
                    cmds.append(f"fill {wx} {G} {z0} {wx} {G} {z1} minecraft:water")
            elif plant == "kelp":
                cmds += [f"fill {x0} {G} {z0} {x1} {G} {z1} minecraft:sand",
                         f"fill {x0} {G+1} {z0} {x1} {G+28} {z1} minecraft:water"]
                for kx in range(0, 16, 2):
                    for kz in range(0, 16, 2):
                        cmds.append(f"setblock {x0+kx} {G+1} {z0+kz} minecraft:kelp[age=0]")
            elif plant == "sapling":
                cmds += [f"fill {x0} {G} {z0} {x1} {G} {z1} minecraft:grass_block",
                         f"fill {x0} {G+1} {z0} {x1} {G+4} {z1} minecraft:air",
                         f"fill {x0} {G+1} {z0} {x1} {G+1} {z1} minecraft:oak_sapling[stage=0]",
                         f"fill {x0} {G+3} {z0} {x1} {G+3} {z1} minecraft:glass"]
    return sum(counts(cmds, timeout=7200))


def read():
    vals = []
    for i in range(NCX):
        for j in range(NCZ):
            ch = anvil.chunk_nbt(WORLD, BX // 16 + i, BZ // 16 + j)
            sy = (G + 1) // 16
            pal, idx = anvil.section_states(ch, sy)
            # kelp columns may span two sections
            pal2 = idx2 = None
            if plant == "kelp":
                r2 = anvil.section_states(ch, sy + 1)
                pal2, idx2 = r2 if r2 else (None, None)
            for x in range(16):
                for z in range(16):
                    if plant == "wheat":
                        n, p = pal[idx[((G + 1) & 15) * 256 + z * 16 + x]]
                        vals.append(int(p.get("age", -1)) if n == "minecraft:wheat" else -1)
                    elif plant == "sapling":
                        n, p = pal[idx[((G + 1) & 15) * 256 + z * 16 + x]]
                        vals.append(int(p.get("stage", 0)) if n == "minecraft:oak_sapling" else -1)
                    else:
                        if x % 2 or z % 2:
                            continue
                        h = 0
                        for dy in range(0, 28):
                            y = G + 1 + dy
                            P, I = (pal, idx) if (y >> 4) == sy else (pal2, idx2)
                            if P is None:
                                break
                            n, p = P[I[(y & 15) * 256 + z * 16 + x]]
                            if n in ("minecraft:kelp", "minecraft:kelp_plant"):
                                h += 1
                            else:
                                break
                        vals.append(h)
    return vals


run([f"forceload add {BX} {BZ} {BX+16*NCX-1} {BZ+16*NCZ-1}"])
assert ready(BX, BZ) and ready(BX + 16 * NCX - 1, BZ + 16 * NCZ - 1)
run(["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "time set 6000", "weather clear 1000000"])
t0 = time.time()
n = build()
print(f"built {n} changes in {time.time()-t0:.0f}s", flush=True)
run([f"gamerule random_tick_speed {speed}"])
t0 = time.time()
g = advance(ticks)
run(["gamerule random_tick_speed 0"])
print(f"advanced {g} ticks in {time.time()-t0:.0f}s", flush=True)
save()
vals = read()
v = [x for x in vals if x >= 0]
res = {"target": T, "plant": plant, "speed": speed, "ticks": ticks, "n": len(v), "invalid": len(vals) - len(v), "hist": {k: v.count(k) for k in sorted(set(v))}}
if plant == "wheat":
    res["mean_age"] = sum(v) / len(v)
elif plant == "kelp":
    res["mean_growth"] = sum(x - 1 for x in v) / len(v)
else:
    res["stage1"] = sum(1 for x in v if x >= 1) / len(v)
json.dump(res, open(OUT, "w"))
print(json.dumps(res))
