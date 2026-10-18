"""wp78 item 5a: the desert pyramid at (-1680,~,-2304) on both servers (par.sh): census (tnt, chests, plate, suspicious sand), then a survival player drops onto the pressure plate;
the TNT trap, the player's health, what is left and what lies on the ground. usage: p78_desert.py [X Z]"""
import collections, hashlib, sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
X, Z = (int(sys.argv[1]), int(sys.argv[2])) if len(sys.argv) > 2 else (-1680, -2304)
def say(*x): print(*x, flush=True)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing true", show=False); sv("gamerule tnt_explodes true", show=False)
sv(f"gamemode creative {ME}", show=False); sv(f"effect give {ME} minecraft:slow_falling 600 0 true", show=False)
sv(f"execute in minecraft:overworld run tp {ME} {X} 100 {Z}", show=False)
time.sleep(25)
g = a.ground(X, Z, 140)
say("ground at the centre", g)
R = 28
census = {}
for nm in ("tnt", "chest", "stone_pressure_plate", "suspicious_sand", "chiseled_sandstone", "orange_terracotta", "blue_terracotta", "cut_sandstone", "sandstone"):
    r = sorted(map(tuple, a.findbox(nm, X - R, g - 45, Z - R, X + R, g + 20, Z + R, 3000)))
    census[nm] = r
    say(f"{nm:22s} {len(r):5d} digest {hashlib.md5(str(r).encode()).hexdigest()[:8]}", r[:4] if len(r) < 40 else "")
plates = census["stone_pressure_plate"]
if not plates:
    say("no pressure plate"); sys.exit()
px, py, pz = plates[0]
say("plate at", (px, py, pz), "block", a.block(px, py, pz)[:60], "tnt below:", [t for t in census["tnt"] if t[1] < py][:4])
# the trap: a survival player on the plate
sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
for slot, it in (("head", "diamond_helmet"), ("chest", "diamond_chestplate"), ("legs", "diamond_leggings"), ("feet", "diamond_boots")):
    sv(f"item replace entity {ME} armor.{slot} with minecraft:{it}[enchantments={{\"minecraft:blast_protection\":4}}]", show=False)
sv(f"effect give {ME} minecraft:resistance 99 1 true", show=False)
sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
before = {nm: len(a.findbox(nm, X - R, g - 45, Z - R, X + R, g + 20, Z + R, 3000)) for nm in ("tnt", "chest", "sandstone", "cut_sandstone", "chiseled_sandstone", "orange_terracotta")}
sv(f"execute in minecraft:overworld run tp {ME} {px + 0.5} {py + 1.5} {pz + 0.5}", show=False)
sv(f"gamemode survival {ME}", show=False)
t0 = time.time()
last = None
for s in range(120):
    time.sleep(0.25)
    es = a.state(40)
    cnt = collections.Counter(e["t"] for e in es["entities"] if e["t"] in ("tnt", "item"))
    row = (dict(cnt), round(es["health"], 1), a.block(px, py, pz)[:40])
    if row != last:
        say(round(time.time() - t0, 2), *row)
    last = row
time.sleep(3)
after = {nm: len(a.findbox(nm, X - R, g - 45, Z - R, X + R, g + 20, Z + R, 3000)) for nm in before}
say("blocks before", before)
say("blocks after ", after)
items = sv("execute as @e[type=item] run data get entity @s Item.id", show=False, wait=1.0)
say("items on the ground:", dict(collections.Counter(re.findall(r'"(minecraft:[a-z_]+)"', items))))
shot("pyramid_" + T)
