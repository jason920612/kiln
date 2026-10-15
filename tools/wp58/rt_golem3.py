"""wp66 item 1c: the big pen of rt_golem without the night: the villagers' last-slept is set by data and it is morning, a zombie in view.
usage: TARGET=K|V rt_golem3.py TICKS PENS OUT.json [nobeds]"""
import sys, json
from p66rt import *
import rt_golem as g

if __name__ == "__main__":
    ticks, pens, outp = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    g.build(pens)
    if "nobeds" in sys.argv:
        for pen in range(pens):
            run([f"fill {g.at(pen,0,1,0)} {g.at(pen,8,1,8)} minecraft:air"])
    run(["time set 1000"])
    gt = gametime()
    run([f'execute as @e[type=minecraft:villager,tag=wp66] run data modify entity @s Brain.memories."minecraft:last_slept" set value {{value:{gt}L}}'])
    g.zombies(pens, True)
    run(["gamerule advance_time true", "data remove storage wp66:log golems", "scoreboard players set #n wp66 0"])
    g0 = gametime()
    n = advance(ticks)
    out = run(["data get storage wp66:log golems", "scoreboard players get #n wp66"], timeout=600)
    rows = g.pattern_rows(" ".join(out))
    json.dump({"target": T, "ticks": n, "pens": pens, "g_start": g0, "golems": rows}, open(outp, "w"))
    print(f"TARGET={T}: {len(rows)} golems in {n} ticks, {pens} pens; {out[-1][:60]}")
