"""wp81 item 5: the cartographer's explorer map trades, Kiln (K) against vanilla (V). A trade's map comes from the `given_item_modifier` of its villager_trade JSON (an
`exploration_map` loot function, then a filter that discards what has no `map_id`) applied to the item it `gives` (e.g. `minecraft:ocean_monument_map`) with the villager's
position as the origin. A data pack `p81` holds one loot table per trade with those very functions, so `execute positioned X Y Z run loot give W0 loot p81:<trade>` makes the
same map; the item (`map_id`, `map_decorations`, name, lore) and the saved map (centre, scale, decorations, the biome preview colours) are compared.
usage: maps81.py     needs both servers (sv.sh; started AFTER this script made the pack: it writes ~/wp81/<K|V>/world/datapacks/p81) and the bot W0"""
import glob, gzip, hashlib, json, os, re, struct, sys, time
from lk import *

WORK = os.path.expanduser("~/kiln/work/generated/data/minecraft/villager_trade/cartographer")
TRADES = [p for p in sorted(glob.glob(WORK + "/*/*map*.json")) if "given_item_modifier" in open(p).read()]
POS = [(0, 64, 0), (3000, 64, 3000), (-5000, 64, 2000), (1200, 64, -4400)]


def make_pack():
    for t in "KV":
        base = os.path.expanduser(f"~/wp81/{t}/world/datapacks/p81")
        os.makedirs(base + "/data/p81/loot_table", exist_ok=True)
        json.dump({"pack": {"description": "p81", "min_format": 121, "max_format": 121}}, open(base + "/pack.mcmeta", "w"))
        for p in TRADES:
            j = json.load(open(p))
            name = os.path.basename(p)[:-5]
            # (As the 26.x loot tables write them: `modifier`, not `functions`; the trade's own item modifiers on the item it gives.)
            table = {"type": "minecraft:chest", "pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": j["gives"]["id"], "modifier": j["given_item_modifier"]}]}]}
            json.dump(table, open(f"{base}/data/p81/loot_table/{name}.json", "w"))


class NBT:
    def __init__(self, b):
        self.b, self.i = b, 0

    def u(self, fmt):
        n = struct.calcsize(fmt)
        v = struct.unpack_from(fmt, self.b, self.i)[0]
        self.i += n
        return v

    def string(self):
        n = self.u(">H")
        s = self.b[self.i:self.i + n].decode("utf-8", "replace")
        self.i += n
        return s

    def payload(self, t):
        if t == 1: return self.u(">b")
        if t == 2: return self.u(">h")
        if t == 3: return self.u(">i")
        if t == 4: return self.u(">q")
        if t == 5: return round(self.u(">f"), 4)
        if t == 6: return self.u(">d")
        if t == 7:
            n = self.u(">i"); v = self.b[self.i:self.i + n]; self.i += n; return bytes(v)
        if t == 8: return self.string()
        if t == 9:
            et = self.u(">b"); n = self.u(">i"); return [self.payload(et) for _ in range(n)]
        if t == 10:
            d = {}
            while True:
                tt = self.u(">b")
                if tt == 0: return d
                k = self.string()
                d[k] = self.payload(tt)
        if t == 11:
            n = self.u(">i"); return [self.u(">i") for _ in range(n)]
        if t == 12:
            n = self.u(">i"); return [self.u(">q") for _ in range(n)]
        raise ValueError(t)


def read_map(path):
    raw = gzip.open(path).read()
    n = NBT(raw)
    t = n.u(">b")
    n.string()
    return n.payload(t)


def clean(v):
    if isinstance(v, bytes):
        return "colors:%d:%s" % (len(v), hashlib.sha1(v).hexdigest()[:12])
    if isinstance(v, dict):
        return {k: clean(x) for k, x in v.items()}
    if isinstance(v, list):
        return [clean(x) for x in v]
    return v


if len(sys.argv) > 1 and sys.argv[1] == "pack":
    make_pack()
    print("pack written for", [os.path.basename(p) for p in TRADES])
    sys.exit()

for t in "KV":
    if t == "V":
        w(t, "op W0")
    w(t, "gamemode creative W0")
    w(t, "reload")
time.sleep(6)
for t in "KV":
    w(t, "datapack enable \"file/p81\"")
time.sleep(6)
for t in "KV":
    print(t, ask(t, "datapack list enabled", wait=0.5).strip()[-200:])
print("trades:", [os.path.basename(p)[:-5] for p in TRADES])
for p in TRADES:
    name = os.path.basename(p)[:-5]
    for (x, y, z) in POS:
        res = {}
        for t in "KV":
            w(t, "clear W0")
            time.sleep(0.2)
            w(t, f"execute in minecraft:overworld positioned {x} {y} {z} run loot give W0 loot p81:{name}")
            time.sleep(1.5)
            inv = ask(t, "data get entity W0 Inventory", wait=0.4)
            res[t] = re.sub(r"^.*?(\[\{|\[\])", r"\1", inv.strip().replace("\n", " "), count=1) if "[" in inv else inv.strip()[-200:]
        same = "SAME" if re.sub(r"\s+", "", res["K"]) == re.sub(r"\s+", "", res["V"]) else "DIFF"
        print(name, (x, z), same)
        if same == "DIFF":
            print("   K", res["K"][:700])
            print("   V", res["V"][:700])
        sys.stdout.flush()
for t in "KV":
    w(t, "save-all flush")
time.sleep(6)
for t in "KV":
    base = os.path.expanduser(f"~/wp81/{t}/world")
    files = sorted(glob.glob(base + "/data/map_*.dat") + glob.glob(base + "/data/minecraft/maps/*.dat"))
    print(t, "map files:", [os.path.relpath(f, base) for f in files][:6], "...", len(files))
kf = sorted(glob.glob(os.path.expanduser("~/wp81/K/world/data/**/*.dat"), recursive=True))
vf = sorted(glob.glob(os.path.expanduser("~/wp81/V/world/data/**/*.dat"), recursive=True))
def key(f):
    return os.path.basename(f)
kd = {key(f): f for f in kf if "map" in f or "maps" in f}
vd = {key(f): f for f in vf if "map" in f or "maps" in f}
diffs = 0
for k in sorted(set(kd) | set(vd)):
    if k not in kd or k not in vd:
        print("only on one side:", k, "K" if k in kd else "V")
        diffs += 1
        continue
    a, b = clean(read_map(kd[k])), clean(read_map(vd[k]))
    da, db = a.get("data", a), b.get("data", b)
    keep = lambda d: {x: d[x] for x in d if x in ("xCenter", "zCenter", "scale", "dimension", "locked", "trackingPosition", "unlimitedTracking", "banners", "frames", "colors", "decorations")}
    if keep(da) != keep(db):
        diffs += 1
        print("MAP", k, "differs")
        for x in sorted(set(keep(da)) | set(keep(db))):
            if keep(da).get(x) != keep(db).get(x):
                print("   ", x, "K", str(keep(da).get(x))[:200], " V", str(keep(db).get(x))[:200])
print("saved maps compared:", len(set(kd) & set(vd)), "differences:", diffs)
