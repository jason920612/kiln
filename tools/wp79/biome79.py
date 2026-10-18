"""wp79 item 3: do Kiln (K) and vanilla (V) agree on `level.getBiome(pos)` (the noise-shifted lookup the spawner uses) in the nether around the roof farm?
Both servers (seed 77, fresh or kept) are asked `execute in minecraft:the_nether if biome X Y Z <biome>` on a grid; every answer is compared.
usage: biome79.py [STEP] [HALF]    grid step (default 8) and half width (default 96) around the farm (-64, 64)"""
import re, sys, time
from lk import *

STEP = int(sys.argv[1]) if len(sys.argv) > 1 else 8
HALF = int(sys.argv[2]) if len(sys.argv) > 2 else 96
CX, CZ = -64, 64
BIOMES = ["nether_wastes", "basalt_deltas", "soul_sand_valley", "crimson_forest", "warped_forest"]
YS = [20, 50, 80, 110]


def prep(t):
    w(t, "execute in minecraft:the_nether run forceload add %d %d %d %d" % (CX - HALF - 16, CZ - HALF - 16, CX + HALF + 16, CZ + HALF + 16))


for t in "KV":
    prep(t)
time.sleep(60)
bad = 0
total = 0
by_biome = {t: {} for t in "KV"}
for y in YS:
    for x in range(CX - HALF, CX + HALF + 1, STEP):
        for z in range(CZ - HALF, CZ + HALF + 1, STEP):
            res = {}
            for t in "KV":
                found = None
                for b in BIOMES:
                    out = ask(t, f"execute in minecraft:the_nether if biome {x} {y} {z} minecraft:{b}", wait=0.02)
                    if "Test passed" in out or "success" in out.lower() and "fail" not in out.lower():
                        found = b
                        break
                res[t] = found
            total += 1
            for t in "KV":
                by_biome[t][res[t]] = by_biome[t].get(res[t], 0) + 1
            if res["K"] != res["V"]:
                bad += 1
                print("DIFF", x, y, z, res, flush=True)
print("points", total, "different", bad)
print("K", by_biome["K"])
print("V", by_biome["V"])
