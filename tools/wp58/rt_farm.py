"""wp66 item 3: the random-tick sampling seen through farmland moisture (each random tick without water lowers it by one, 7 -> 0, so
7 - moisture is the number of random ticks a block got). A section of farmland per chunk, `tick step 1`, `save-all flush`, read the world
files: hits per position per tick, no block changes after the build.
usage: TARGET=K|V rt_farm.py SPEED TICKS NCX NCZ [Y0 [OUT.npy]]"""
import sys, pickle
from p66rt import *
import anvil

speed, ticks, NCX, NCZ = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
Y0 = int(sys.argv[5]) if len(sys.argv) > 5 else -48          # a section's bottom: -48 is section -3, just above the flat world's grass
OUT = sys.argv[6] if len(sys.argv) > 6 else None
BX, BZ = 1024, 1024
WORLD = DIR / "world"
SY = Y0 // 16


def save():
    """save-all flush and wait until the region file(s) of the area were rewritten."""
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
            x, z = BX + 16 * i, BZ + 16 * j
            cmds.append(f"fill {x} {Y0} {z} {x+15} {Y0+15} {z+15} minecraft:air")
            for k in range(8):
                cmds.append(f"fill {x} {Y0+2*k} {z} {x+15} {Y0+2*k} {z+15} minecraft:farmland[moisture=7]")
    t0 = time.time()
    res = counts(cmds, timeout=3600)
    return sum(res), time.time() - t0


def read():
    """Hits per position as bytes (chunk-major, then y*256+z*16+x within the chunk): 7 - moisture, 9 for 'turned to dirt / not farmland'."""
    out = bytearray()
    for i in range(NCX):
        for j in range(NCZ):
            ch = anvil.chunk_nbt(WORLD, BX // 16 + i, BZ // 16 + j)
            pal, idx = anvil.section_states(ch, SY)
            hit = [9 if n != "minecraft:farmland" else 7 - int(p.get("moisture", 7)) for n, p in pal]
            out += bytes(hit[k] for k in idx)
    return bytes(out)


run([f"forceload add {BX} {BZ} {BX+16*NCX-1} {BZ+16*NCZ-1}"])
assert ready(BX, BZ) and ready(BX + 16 * NCX - 1, BZ + 16 * NCZ - 1)
run(["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "time set day", "weather clear 1000000"])
n, dt = build()
print(f"built {n} farmland in {dt:.0f}s", flush=True)
save()
prev = read()
assert set(prev) <= {0, 9}, set(prev)
hist = []
run([f"gamerule random_tick_speed {speed}"])
for t in range(ticks):
    step(1)
    save()
    cur = read()
    d = bytes((c - p) if (c < 8 and p < 8) else 255 for c, p in zip(cur, prev))
    hist.append(d)
    valid = [x for x in d if x != 255]
    hh = [0] * 8
    for x in valid:
        hh[x] += 1
    print(f"tick {t}: mean hits/position {sum(valid)/max(1,len(valid)):.4f} (valid {len(valid)/len(d):.3f}) hist {hh}", flush=True)
    prev = cur
    if sum(1 for c in cur if 5 <= c <= 8) / (len(cur) // 2) > 0.3 and t + 1 < ticks:
        run(["gamerule random_tick_speed 0"])
        n, dt = build()
        print(f"rebuilt {n} in {dt:.0f}s", flush=True)
        save()
        prev = read()
        run([f"gamerule random_tick_speed {speed}"])
if OUT:
    pickle.dump({"speed": speed, "target": T, "ncx": NCX, "ncz": NCZ, "y0": Y0, "ticks": hist}, open(OUT, "wb"))
