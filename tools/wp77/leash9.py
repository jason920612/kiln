"""wp77 item 9: a NoAI cow pulled by a leash, Motion and Pos per tick on Kiln (K) and vanilla (V) in lock step.
usage: leash9.py [NOAI=1] [TICKS=40]"""
import re, sys, time
from lk import *

noai = (sys.argv[1] if len(sys.argv) > 1 else "1") == "1"
ticks = int(sys.argv[2]) if len(sys.argv) > 2 else 40
ME = "W0"


def setup(t):
    for c in ("gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false", "difficulty peaceful",
              "forceload add 0 0 48 24", "kill @e[type=!player]"):
        w(t, c)
    time.sleep(2.0)
    for c in ("fill -2 199 -2 44 199 20 stone", "fill -2 200 -2 44 215 20 air", f"tp {ME} 20 201 12",
              "setblock 30 200 5 oak_fence", "kill @e[type=!player]"):
        w(t, c)
        time.sleep(0.3)
    time.sleep(1.0)
    w(t, "summon minecraft:leash_knot 30 200 5")
    time.sleep(0.3)
    nbt = '{Tags:["c9"],leash:[I;30,200,5]' + (",NoAI:1b" if noai else "") + "}"
    w(t, f"summon minecraft:cow 5.5 200 5.5 {nbt}")
    time.sleep(0.5)


def motion(t):
    out = ask(t, "data get entity @e[tag=c9,limit=1] Motion", wait=0.25)
    pos = ask(t, "data get entity @e[tag=c9,limit=1] Pos", wait=0.25)
    m = re.findall(r"\[([^\]]*)\]", out)
    p = re.findall(r"\[([^\]]*)\]", pos)
    return (m[-1] if m else out.strip()[-80:]), (p[-1] if p else pos.strip()[-80:])


for t in "KV":
    setup(t)
both("tick freeze")
time.sleep(0.5)
rows = []
for i in range(ticks):
    both("tick step 1")
    time.sleep(0.2)
    rows.append((i, motion("K"), motion("V")))
same = 0
for i, k, v in rows:
    mark = "==" if k == v else "!="
    same += k == v
    print(i, mark, "K", k[0][:90], "|", k[1][:60], "   V", v[0][:90], "|", v[1][:60])
print("same", same, "of", len(rows))
