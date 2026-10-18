"""wp79: how far from a player do entities tick (the simulation distance in chunks), Kiln (K) against vanilla (V)? Baby cows (Age -24000, NoGravity) are summoned at the
middle of the chunks 0..9 east of the player's chunk, with the player at (8.5, y, 8.5) of its chunk; the `Age` of each after N ticks says whether its chunk ticked
(AgeableMob.aiStep adds 1 per tick). Both servers: seed 77, view distance VIEW, simulation distance SIM (sv.sh). usage: simradius79.py [TICKS]"""
import re, sys, time
from lk import *

TICKS = int(sys.argv[1]) if len(sys.argv) > 1 else 40
X0, Z0, Y = 1000, 1000, 220


def ages(t):
    res = {}
    for d in range(0, 10):
        out = ask(t, f"data get entity @e[tag=rt{d},limit=1] Age", wait=0.2)
        m = re.search(r"(-?\d+)\s*$", out.strip().splitlines()[-1]) if out.strip() else None
        m2 = re.findall(r"(-?\d+)(?!.*\d)", out)
        res[d] = m2[-1] if m2 else None
    return res


for t in "KV":
    w(t, "gamerule spawn_mobs false")
    w(t, "gamemode creative W0")
    w(t, f"tp W0 {X0 + 8.5} {Y} {Z0 + 8.5}")
time.sleep(25)
for t in "KV":
    w(t, "kill @e[type=minecraft:cow]")
    for d in range(10):
        w(t, f"summon cow {X0 + 8.5 + 16 * d} {Y - 40} {Z0 + 8.5} {{Age:-24000,NoGravity:1b,PersistenceRequired:1b,Tags:[\"rt{d}\"]}}")
time.sleep(1)
before = {t: ages(t) for t in "KV"}
for t in "KV":
    w(t, f"tick sprint {TICKS}")
time.sleep(0.5 + TICKS * 0.06)
time.sleep(2)
after = {t: ages(t) for t in "KV"}
print("distance (chunks east of the player's chunk): Age before -> after")
for d in range(10):
    print(d, "K", before["K"][d], "->", after["K"][d], "  V", before["V"][d], "->", after["V"][d])
