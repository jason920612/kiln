"""wp74: on foot in the nether (par.sh): a fortress and a bastion remnant. A real client lands next to the structure, walks (digging when stuck) a loop
through it, and logs what it meets: entity types seen, damage taken, blocks, the server's mspt.  usage: p74_walk.py [fortress|bastion_remnant]"""
import sys, collections, math
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
WHAT = sys.argv[1] if len(sys.argv) > 1 else "fortress"
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs true", show=False); sv("gamerule mob_griefing true", show=False); sv("difficulty normal", show=False)
sv(f"gamemode survival {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"give {ME} minecraft:iron_sword", show=False); sv(f"give {ME} minecraft:iron_pickaxe", show=False); sv(f"give {ME} minecraft:cooked_beef 32", show=False)
for slot in ("head", "chest", "legs", "feet"):
    sv(f"item replace entity {ME} armor.{slot} with minecraft:iron_{ {'head':'helmet','chest':'chestplate','legs':'leggings','feet':'boots'}[slot] }", show=False)
sv(f"effect give {ME} minecraft:fire_resistance 99999 0 true", show=False)
out = sv(f"execute in minecraft:the_nether positioned 0 64 0 run locate structure minecraft:{WHAT}", wait=8, show=False)
m = re.search(r"\[(-?\d+), (-?\d+|~), (-?\d+)\]", out)
x, z = int(m.group(1)), int(m.group(3))
print(T, WHAT, "at", x, z, flush=True)
sv(f"forceload add {x - 32} {z - 32} {x + 32} {z + 32}", show=False)
sv(f"execute in minecraft:the_nether run tp {ME} {x}.5 110 {z}.5", show=False)
sv(f"effect give {ME} minecraft:slow_falling 12 0 true", show=False)
time.sleep(20)
s0 = a.state(0)
print(T, "landed at", [round(s0[k], 1) for k in "xyz"], "health", s0["health"], flush=True)
seen = collections.Counter()
hp_min = 20.0
t0 = time.time()
route = [(x + 24, z), (x + 24, z + 24), (x, z + 24), (x - 24, z + 24), (x - 24, z), (x, z)]
for (tx, tz) in route:
    ok = go(a, tx + 0.5, tz + 0.5, timeout=40, sprint=True, tol=3.0)
    st = a.state(40)
    for e in st["entities"]:
        seen[e["t"]] += 1
    hp_min = min(hp_min, st["health"])
    if st["health"] < 8:
        sv(f"effect give {ME} minecraft:instant_health 1 3 true", show=False)
    alive_auto(a)
    print(T, "waypoint", (tx, tz), "ok", ok, "pos", [round(st[k]) for k in "xyz"], "health", round(st["health"], 1), flush=True)
print(T, "RESULT", WHAT, "entity sightings (summed over waypoints):", dict(seen), "min health", hp_min, "secs", round(time.time() - t0), flush=True)
print(T, "mspt", mspt())
