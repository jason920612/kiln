"""wp74: villages on identical worlds, Kiln against vanilla: cat spawning (CatSpawner), villager / golem / cat counts over a day.
For each of several villages the player hovers above it (creative, slow falling) while `tick sprint` runs ROUNDS x ROUND_TICKS ticks; counts after every round.
usage: p74_village.py [NVILLAGES] [ROUNDS] [ROUND_TICKS]"""
import sys, threading, json
from lock import *

NV = int(sys.argv[1]) if len(sys.argv) > 1 else 4
ROUNDS = int(sys.argv[2]) if len(sys.argv) > 2 else 12
RT = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
SPOTS = [(9000, 0), (0, 9000), (-9000, 0), (0, -9000), (9000, 9000), (-9000, 9000), (9000, -9000), (-9000, -9000), (14000, 0), (0, 14000)]
ME = "KilnTest"


def box(x, z, h=96):
    return f"x={x - h},y=-64,z={z - h},dx={2 * h},dy=400,dz={2 * h}"


def go(t, res):
    if t == "V":
        w(t, f"op {ME}")
    for g in ("spawn_mobs true", "spawn_monsters false", "advance_weather false", "mob_griefing false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal")
    w(t, f"gamemode creative {ME}")
    villages = []
    seen = set()
    for (sx, sz) in SPOTS:
        v = locate_xz(t, "#minecraft:village", sx, sz)
        if v and v not in seen and all(abs(v[0] - a) + abs(v[1] - b) > 400 for a, b in seen):
            seen.add(v); villages.append(v)
        if len(villages) >= NV:
            break
    res[t] = {"villages": villages, "rows": {}}
    for (x, z) in villages:
        w(t, f"time set 1000")
        w(t, f"execute in minecraft:overworld run tp {ME} {x} 140 {z}")
        w(t, f"effect give {ME} minecraft:slow_falling 99999 0 true")
        w(t, f"forceload add {x - 48} {z - 48} {x + 48} {z + 48}")
        wait_loaded(t, x + 40, 100, z + 40)
        wait_loaded(t, x - 40, 100, z - 40)
        time.sleep(5)
        rows = []
        for r in range(ROUNDS):
            sprint(t, RT)
            b = box(x, z)
            row = {"cat": count(t, f"@e[type=cat,{b}]"), "villager": count(t, f"@e[type=villager,{b}]"), "golem": count(t, f"@e[type=iron_golem,{b}]"),
                   "jobless": count(t, f'@e[type=villager,{b},nbt={{VillagerData:{{profession:"minecraft:none"}}}}]'),
                   "baby": count(t, f"@e[type=villager,{b},nbt={{Age:-1}}]") if False else 0}
            rows.append(row)
        print(t, (x, z), rows, flush=True)
        res[t]["rows"][f"{x},{z}"] = rows
        w(t, f"forceload remove {x - 48} {z - 48} {x + 48} {z + 48}")


res = {}
th = [threading.Thread(target=go, args=(t, res)) for t in "KV"]
[x.start() for x in th]; [x.join() for x in th]
print("RESULT", json.dumps(res))
