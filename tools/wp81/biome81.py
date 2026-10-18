"""wp81 item 3: do Kiln (K) and vanilla (V) agree on `level.getBiome(pos)` (the noise-shifted lookup the spawner uses) in the nether around the roof farm?
Both servers (seed 77, fresh or kept) are asked `execute in minecraft:the_nether if biome X Y Z <biome>` on a grid; every answer is compared.
usage: biome81.py [STEP] [HALF]    grid step (default 8) and half width (default 96) around the farm (-64, 64); BIOME_VECTORS=<file> writes vanilla's answers (the Rust test reads them)"""
import os, re, sys, time
from lk import *
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from biomes_saved import stored_biome

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
agree = {"K": 0, "V": 0}
by_biome = {t: {} for t in "KV"}
rows = []
for y in YS:
    for x in range(CX - HALF, CX + HALF + 1, STEP):
        for z in range(CZ - HALF, CZ + HALF + 1, STEP):
            res = {}
            for t in "KV":
                n = logpos(t)
                for b in BIOMES:
                    w(t, f"execute in minecraft:the_nether if biome {x} {y} {z} minecraft:{b}")
                time.sleep(0.2)
                out = since(t, n)
                answers = [("pass" in l.lower()) for l in out.splitlines() if "conditional" in l or "Test " in l]
                found = None
                if len(answers) == len(BIOMES):
                    for b, a in zip(BIOMES, answers):
                        if a:
                            found = b
                            break
                else:
                    found = "?%d" % len(answers)
                res[t] = found
            total += 1
            st = stored_biome(os.path.expanduser("~/wp81/K/world"), "dimensions/minecraft/the_nether", x, y, z)
            st = st.replace("minecraft:", "") if st else None
            res["S"] = st
            for t in "KV":
                agree[t] += res[t] == st
            rows.append({"x": x, "y": y, "z": z, "vanilla": res["V"], "stored": st})
            for t in "KV":
                by_biome[t][res[t]] = by_biome[t].get(res[t], 0) + 1
            if res["K"] != res["V"]:
                bad += 1
                print("DIFF", x, y, z, res, flush=True)
import json
out_path = os.environ.get("BIOME_VECTORS")
if out_path:
    with open(out_path, "w") as f:
        for row in rows:
            f.write(json.dumps(row) + chr(10))
print("points", total, "different", bad, "agree with the stored quart: K", agree["K"], "V", agree["V"])
print("K", by_biome["K"])
print("V", by_biome["V"])
