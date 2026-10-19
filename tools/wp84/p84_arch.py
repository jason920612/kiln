"""wp84 #4: archaeology structures on both servers (same seed): `locate` the structure, fly both real clients there, then what each client sees: the suspicious sand /
gravel blocks, decorated pots, chests, the block entity data (loot table, item) of each. Lock-step lab: one process drives both consoles and both clients (25999 = Kiln, 26000 = vanilla).
usage: python3 p84_arch.py [kinds]   kinds: desert_pyramid,trail_ruins,ocean_ruin_cold,ocean_ruin_warm,desert_well(feature, not locatable),jungle_temple"""
from lab84 import *

KINDS = (sys.argv[1] if len(sys.argv) > 1 else "desert_pyramid,trail_ruins,ocean_ruin_cold,ocean_ruin_warm").split(",")
cl = clients()
WANT = ("suspicious_sand", "suspicious_gravel", "decorated_pot", "chest", "trapped_chest", "barrel", "spawner", "tnt")


def fly(t, x, y, z):
    w(t, "gamemode creative KilnTest")
    w(t, f"execute in minecraft:overworld run tp KilnTest {x} {y} {z}")


for kind in KINDS:
    loc = {}
    for t in "KV":
        loc[t] = locate_xz(t, f"minecraft:{kind}", 0, 0, wait=10)
    print(f"{kind}: locate K {loc['K']} V {loc['V']}", flush=True)
    if not loc["K"] or not loc["V"]:
        continue
    if loc["K"] != loc["V"]:
        print("  (different structure positions: using each server's own)", flush=True)
    found = {}
    for t in "KV":
        x, z = loc[t]
        y = 100
        both = {}
        fly(t, x, y, z)
        time.sleep(1.0)
    time.sleep(int(os.environ.get("ARCH_WAIT", "14")))  # chunks arrive
    for t in "KV":
        x, z = loc[t]
        c = cl[t]
        st = c.state(0)
        print(f"  {t} client at {st['x']:.0f},{st['y']:.0f},{st['z']:.0f}", flush=True)
        # the ground height: go down while the client does not see solid ground
        res = {}
        for w_ in WANT:
            r = c.cmd(f"findbox {w_} {x - 40} -64 {z - 40} {x + 40} 200 {z + 40} 400")
            try:
                pts = json.loads(r)
            except Exception:
                pts = []
            res[w_] = pts
        found[t] = res
    for w_ in WANT:
        k, v = found["K"][w_], found["V"][w_]
        ks = sorted(tuple(p[:3]) if isinstance(p, list) else tuple(p) for p in k) if k and isinstance(k[0], list) else sorted(map(str, k))
        vs = sorted(tuple(p[:3]) if isinstance(p, list) else tuple(p) for p in v) if v and isinstance(v[0], list) else sorted(map(str, v))
        if k or v:
            print(f"  {w_:18s} K {len(k):3d}  V {len(v):3d}  {'SAME positions' if ks == vs else 'different positions'}", flush=True)
            if ks != vs:
                print("     only K:", [p for p in ks if p not in vs][:6], " only V:", [p for p in vs if p not in ks][:6], flush=True)
    # block entity data of the suspicious blocks and pots (same positions only)
    for w_ in ("suspicious_sand", "suspicious_gravel", "decorated_pot"):
        k, v = found["K"][w_], found["V"][w_]
        common = [p for p in k if p in v][:5]
        for p in common:
            x_, y_, z_ = p[:3]
            a = nbt("K", "block", f"{x_} {y_} {z_}")
            b = nbt("V", "block", f"{x_} {y_} {z_}")
            same = re.sub(r"LootTableSeed: -?\d+L?", "", a) == re.sub(r"LootTableSeed: -?\d+L?", "", b)
            print(f"    {w_} {x_} {y_} {z_}: {'SAME' if same else 'DIFF'}  K {a[:200]}  V {b[:200]}", flush=True)
for t in "KV":
    w(t, "gamemode creative KilnTest")
