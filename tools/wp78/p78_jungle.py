"""wp78 item 5d: a jungle temple (placed with /place structure on a stone platform; both servers, par.sh): census of tripwire hooks, dispensers, levers; a survival
player walks over the tripwires (arrows from the dispensers), health and arrows seen."""
import collections, hashlib, sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
X, Z = -304, -1232
def say(*x): print(*x, flush=True)
def O(cmd, wait=0.5): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False)
sv(f"gamemode creative {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"effect give {ME} minecraft:slow_falling 600 0 true", show=False)
O(f"forceload add {X - 25} {Z - 25} {X + 25} {Z + 25}")
O(f"tp {ME} {X} 120 {Z}")
time.sleep(20)
g = a.ground(X, Z, 160)
base = g - 6
say("ground", g)
R = 22
cen = {}
for nm in ("tripwire_hook", "tripwire", "dispenser", "lever", "chest", "sticky_piston", "redstone_wire", "mossy_cobblestone", "chiseled_stone_bricks"):
    rr = sorted(map(tuple, a.findbox(nm, X - R, g - 30, Z - R, X + R, g + 12, Z + R, 2000)))
    cen[nm] = rr
    say(f"{nm:22s} {len(rr):4d}", rr[:6])
hooks = cen["tripwire_hook"]
wires = [w for w in cen["tripwire"] if "hook" not in a.block(*w)]
if not wires:
    sys.exit()
# the tripwire line nearest to the dispensers: pick the first string and its axis
w0 = wires[0]
axis = "x" if any(w[2] == w0[2] and w[0] != w0[0] for w in wires) else "z"
say("first tripwire", w0, "axis", axis, "block", a.block(*w0)[:80])
# stand 3 blocks before it perpendicular to the string, walk across
px, py, pz = w0
perp = (0, 1) if axis == "x" else (1, 0)
start = (px - perp[0] * 3, py, pz - perp[1] * 3)
end = (px + perp[0] * 3, py, pz + perp[1] * 3)
say("start", start, [a.block(start[0], py, start[2])[16:40], a.block(start[0], py + 1, start[2])[16:40]], "end", end)
sv(f"clear {ME}", show=False)
sv(f"effect give {ME} minecraft:resistance 99 0 true", show=False)
sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
O(f"tp {ME} {start[0] + 0.5} {py} {start[2] + 0.5}")
sv(f"gamemode survival {ME}", show=False)
time.sleep(2)
arrows = 0
hp = []
t0 = time.time()
a.key("w", "down") if False else None
a.walk_to(end[0] + 0.5, end[2] + 0.5, timeout=8, tol=0.8)
for s in range(40):
    es = a.state(30)
    hp.append(round(es["health"], 1))
    arrows = max(arrows, len([e for e in es["entities"] if e["t"] == "arrow"]))
    time.sleep(0.25)
say("after the walk: lowest health", min(hp), "arrows seen at once", arrows, "player at", me(a))
say("tripwire state after:", a.block(*w0)[:80], "arrows on the ground:", len([e for e in a.state(30)["entities"] if e["t"] == "arrow"]))
