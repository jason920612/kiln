"""wp66 item 3/4: observer + piston sugar cane farms (the wp65 layout) on the flat ground, UNITS of them, run TICKS game ticks at random_tick_speed SPEED with
`tick sprint`; the harvested canes (item entities) are counted. usage: TARGET=K|V rt_machine.py SPEED TICKS UNITS [X0 Z0]"""
import sys, re
from p66rt import *

speed, ticks, units = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
X0 = int(sys.argv[4]) if len(sys.argv) > 4 else 2000
Z0 = int(sys.argv[5]) if len(sys.argv) > 5 else 2000
Y = -60


def build():
    nz = 4 * units + 4
    run([f"forceload add {X0-4} {Z0-4} {X0+8} {Z0+nz}"])
    assert ready(X0, Z0) and ready(X0 + 8, Z0 + nz)
    run(["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "time set day", "weather clear 1000000", "kill @e[type=item]"])
    cmds = []
    for i in range(units):
        z = Z0 + 4 * i
        cmds += [
            f"setblock {X0} {Y} {z} minecraft:sand",
            f"setblock {X0+1} {Y} {z} minecraft:water",
            f"setblock {X0+1} {Y+1} {z} minecraft:stone",
            f"setblock {X0+2} {Y} {z} minecraft:stone",
            f"setblock {X0+1} {Y} {z-1} minecraft:stone",
            f"setblock {X0+1} {Y} {z+1} minecraft:stone",
            f"setblock {X0} {Y+1} {z} minecraft:sugar_cane",
            f"setblock {X0-1} {Y+2} {z} minecraft:piston[facing=east]",
            f"setblock {X0} {Y+3} {z} minecraft:observer[facing=down]",
            f"setblock {X0} {Y+4} {z} minecraft:stone",
            f"setblock {X0-1} {Y+3} {z} minecraft:stone",
            f"setblock {X0-1} {Y+4} {z} minecraft:redstone_wire",
        ]
    counts(cmds, timeout=3600) if False else run(cmds, timeout=3600)


def items():
    out = run([f"execute as @e[type=minecraft:item,x={X0-4},y={Y-3},z={Z0-4},dx=14,dy=16,dz={4*units+10}] run data get entity @s Item"], timeout=600)
    tot = 0
    for l in out:
        m = re.search(r"count: ?(\d+)", l)
        if m and "sugar_cane" in l:
            tot += int(m.group(1))
    return tot, len(out)


build()
canes = run([f"execute if block {X0} {Y+1} {Z0} minecraft:sugar_cane run say OK"])
print("built", canes, flush=True)
run([f"gamerule random_tick_speed {speed}"])
t0 = time.time()
n = advance(ticks)
run(["gamerule random_tick_speed 0"])
tot, lines = items()
print(f"TARGET={T} speed={speed} ticks={n} units={units}: {tot} canes harvested ({tot/units:.2f} per farm, {tot/n:.4f} per tick; {lines} item lines) [{time.time()-t0:.0f}s]")
