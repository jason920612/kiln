"""wp78 item 1c: soul sand valley and basalt deltas (par.sh): locate the biome (same answer on both?), a creative scout at the spot, block digests, then 100 s with a survival player (resistance, fire resistance):
the largest number of each mob type seen at once."""
import collections, hashlib, re
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
def say(*x): print(*x, flush=True)
def N(cmd, wait=0.4): return sv("execute in minecraft:the_nether run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs true", show=False)
for biome in ("minecraft:soul_sand_valley", "minecraft:basalt_deltas", "minecraft:crimson_forest", "minecraft:warped_forest"):
    o = sv(f"execute in minecraft:the_nether positioned 0 64 0 run locate biome {biome}", show=False, wait=8.0)
    m = re.findall(r"\[(-?\d+), (~|-?\d+), (-?\d+)\]", o)
    say(biome, "->", m[-1] if m else o.strip()[-100:])
    if not m:
        continue
    x, z = int(m[-1][0]), int(m[-1][2])
    sv(f"gamemode creative {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:fire_resistance 3000 0 true", show=False); sv(f"effect give {ME} minecraft:slow_falling 3000 0 true", show=False)
    N(f"tp {ME} {x} 100 {z}")
    time.sleep(30)
    R = 24
    for nm in ("soul_sand", "soul_soil", "basalt", "blackstone", "magma_block", "nether_wart_block", "warped_nylium", "crimson_nylium", "shroomlight", "weeping_vines", "soul_fire", "bone_block"):
        r = sorted(map(tuple, a.findbox(nm, x - R, 20, z - R, x + R, 120, z + R, 3000)))
        if r:
            say(f"   {nm:18s} {len(r):5d} digest {hashlib.md5(str(r).encode()).hexdigest()[:8]}")
    spot = free_spot(a, x, 90, z, r=10, dy=40)
    if spot:
        N(f"tp {ME} {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}")
    sv(f"gamemode survival {ME}", show=False)
    sv(f"effect give {ME} minecraft:resistance 3000 4 true", show=False)
    t0 = time.time()
    mx = collections.Counter()
    while time.time() - t0 < 100:
        for t, n in collections.Counter(e["t"] for e in a.state(64)["entities"] if e["t"] not in ("item", "experience_orb", "small_fireball", "fireball")).items():
            mx[t] = max(mx[t], n)
        alive_auto(a)
        time.sleep(2)
    say("   mobs seen at once within 64 blocks over 100 s:", dict(mx))
