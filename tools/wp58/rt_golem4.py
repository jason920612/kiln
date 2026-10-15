"""wp66 item 1d: where the villagers of the big pen are (their distance to the zombie decides what they see): positions every 50 ticks
after the morning, the golem logger off (zombie in view, last-slept set). usage: TARGET=K|V rt_golem4.py SAMPLES PENS OUT.json"""
import sys, json, re
from p66rt import *
import rt_golem as g

if __name__ == "__main__":
    samples, pens, outp = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    g.build(pens)
    run(["time set 1000"])
    gt = gametime()
    run([f'execute as @e[type=minecraft:villager,tag=wp66] run data modify entity @s Brain.memories."minecraft:last_slept" set value {{value:{gt}L}}'])
    g.zombies(pens, True)
    run(["gamerule advance_time true"])
    rows = []
    for i in range(samples):
        advance(50)
        out = run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Pos"])
        pts = []
        for l in out:
            m = re.search(r"\[(-?[\d.]+)d, (-?[\d.]+)d, (-?[\d.]+)d\]", l)
            if m:
                pts.append([float(m.group(k)) for k in (1, 2, 3)])
        rows.append(pts)
    json.dump({"target": T, "rows": rows, "x0": g.X0, "z0": g.Z0, "pen_dz": g.PEN_DZ}, open(outp, "w"))
    print("TARGET", T, "samples", len(rows), "villagers per sample", len(rows[0]))
