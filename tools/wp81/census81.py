"""wp81: the mobs every dimension holds after the player stood in the nether farm spot for a while (no kills before), by type, Kiln (K) against vanilla (V):
the creature mob cap of the nether (strider, 10 per 289 chunks) is full in vanilla (count 15-16 at every check) and empty in Kiln.
usage: census79.py [WAIT_S]   needs both servers up (sv.sh, seed 77) and the bot W0 in"""
import re, sys, time
from lk import *

WAIT = int(sys.argv[1]) if len(sys.argv) > 1 else 30
TYPES = ["strider", "hoglin", "piglin", "zombified_piglin", "ghast", "magma_cube", "blaze", "skeleton", "wither_skeleton", "enderman", "bat", "cow", "sheep", "pig", "chicken"]
x0, z0 = -64, 64
for t in "KV":
    w(t, "gamerule spawn_mobs false")
    w(t, "gamemode creative W0")
    w(t, f"execute in minecraft:the_nether run tp W0 {x0 + 0.5} 191 {z0 + 0.5}")
time.sleep(WAIT)
for t in "KV":
    w(t, f"execute in minecraft:the_nether run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
time.sleep(20)


def count(t, dim, ty):
    out = ask(t, f"execute in minecraft:{dim} if entity @e[type=minecraft:{ty}]", wait=0.25)
    return out


def n_of(t, dim, ty):
    out = ask(t, f"execute in minecraft:{dim} run kill @e[type=minecraft:{ty}]", wait=0.3)
    m = re.search(r"success\.multiple\[(\d+)\]|Killed (\d+) entities", out)
    if m:
        return int(m.group(1) or m.group(2))
    return 1 if ("success.single" in out or re.search(r"Killed [A-Za-z]", out)) else 0


res = {t: {} for t in "KV"}
for t in "KV":
    for dim in ("overworld", "the_nether", "the_end"):
        for ty in TYPES:
            n = n_of(t, dim, ty)
            if n:
                res[t][(dim, ty)] = n
for key in sorted(set(res["K"]) | set(res["V"])):
    print("%-12s %-18s K %4d  V %4d" % (key[0], key[1], res["K"].get(key, 0), res["V"].get(key, 0)))
