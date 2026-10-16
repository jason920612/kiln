"""wp69: where the villagers and the zombie of one pen are, tick by tick, for the first TICKS ticks after the zombie comes, over DAYS mornings
(TARGET=K|V; env HOMES, ZTP, ALIGN as in rt_farm69). usage: pos_probe.py DAYS TICKS OUT.json"""
import sys, re, json, os
from p69rt import *
import rt_farm69 as g

NUM = re.compile(r"-?\d+\.\d+")


def positions():
    out = run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Pos",
               "execute as @e[type=minecraft:zombie,tag=wp66z] run data get entity @s Pos"])
    rows = []
    for l in out:
        m = re.findall(r"-?\d+\.\d+", l.split("entity data:")[-1])
        if len(m) == 3:
            rows.append([float(v) for v in m])
    return rows


if __name__ == "__main__":
    days, ticks, outp = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    g.build(1)
    if os.environ.get("ALIGN"):
        gt = gametime()
        run([f"time set {11500 + gt % 100}"])
    run(["gamerule advance_time true"])
    res = []
    for day in range(days + 1):
        dt = day_time()
        g.zombies(1, False)
        advance((24000 - dt + 300) % 24000)
        g.zombies(1, True)
        if day == 0:
            advance(11000 - 300 - 1)
            continue
        series = []
        for t in range(ticks):
            advance(1)
            series.append((gametime(), positions()))
        res.append(series)
        print("day", day, len(series), flush=True)
        dt = day_time()
        advance(max(0, 11000 - dt))
    json.dump(res, open(outp, "w"))
