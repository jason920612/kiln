"""wp66 item 1e: where the big pen's villagers stand, counted on the server every 20 ticks (x bin of each villager and whether the zombie is
within 8 blocks), over TICKS morning ticks with the last-slept set; and the golems of the same run. usage: TARGET=K|V rt_golem5.py TICKS PENS OUT.json"""
import sys, json, re
from p66rt import *
import rt_golem as g

if __name__ == "__main__":
    ticks, pens, outp = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    g.build(pens)
    run(["time set 1000", "scoreboard players set #20 wp66 20", f"scoreboard players set #x0 wp66 {g.X0}", "scoreboard players set #on wp66 0"])
    gt = gametime()
    run([f'execute as @e[type=minecraft:villager,tag=wp66] run data modify entity @s Brain.memories."minecraft:last_slept" set value {{value:{gt}L}}'])
    g.zombies(pens, True)
    names = ["bm", "b0", "b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8", "b9", "near", "samples"]
    run([f"scoreboard players set #{n} wp66 0" for n in names] + ["gamerule advance_time true", "data remove storage wp66:log golems", "scoreboard players set #n wp66 0", "scoreboard players set #on wp66 1"])
    n = advance(ticks)
    out = run([f"scoreboard players get #{x} wp66" for x in names] + ["data get storage wp66:log golems", "scoreboard players get #n wp66"], timeout=600)
    vals = {}
    for nme, l in zip(names, out):
        m = re.search(r"has (\d+)|, (\d+), \[", l)
        vals[nme] = int(m.group(1) or m.group(2)) if m else None
    rows = g.pattern_rows(" ".join(out))
    json.dump({"target": T, "ticks": n, "pens": pens, "bins": vals, "golems": rows}, open(outp, "w"))
    print(f"TARGET={T}: {len(rows)} golems; bins {vals}")
