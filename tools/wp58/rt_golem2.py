"""wp66 item 1b: the golem farm's logic without the walking: three villagers (last slept = now, set by data) in a 3x3 pen, a zombie in
view 3 blocks away behind a fence, all daytime. Every spawn is logged by wp66:tick (golems killed at once).
usage: TARGET=K|V rt_golem2.py TICKS PENS OUT.json"""
import sys, json, os
from p66rt import *
import rt_golem as g

X0, Z0, F = g.X0, g.Z0 + 300, g.F   # a different area
PEN_DZ = 40


def at(pen, dx, dy, dz):
    return f"{X0+dx} {F+dy} {Z0+pen*PEN_DZ+dz}"


def build(pens):
    run([f"forceload add {X0-12} {Z0-12} {X0+24} {Z0+pens*PEN_DZ+20}"])
    assert ready(X0, Z0) and ready(X0 + 24, Z0 + pens * PEN_DZ + 20) and ready(X0 - 12, Z0 - 12)
    run(["tick freeze", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "gamerule mob_drops false",
         "difficulty normal", "weather clear 1000000", "time set 1000", "kill @e[type=!player]", "scoreboard objectives add wp66 dummy"], timeout=300)
    for pen in range(pens):
        run([
            f"fill {X0-12} {F} {Z0+pen*PEN_DZ-12} {X0+24} {F} {Z0+pen*PEN_DZ+20} minecraft:stone",
            f"fill {X0-12} {F+1} {Z0+pen*PEN_DZ-12} {X0+24} {F+8} {Z0+pen*PEN_DZ+20} minecraft:air",
            f"fill {at(pen,-1,1,-1)} {at(pen,8,3,3)} minecraft:stone_bricks",
            f"fill {at(pen,0,1,0)} {at(pen,2,2,2)} minecraft:air",
            f"fill {at(pen,4,1,0)} {at(pen,7,2,2)} minecraft:air",
        ] + [f"setblock {at(pen,3,1,z)} minecraft:oak_fence" for z in range(3)] + [f"setblock {at(pen,3,2,z)} minecraft:air" for z in range(3)], timeout=3600)
    run([f'summon minecraft:villager {X0+1}.5 {F+1} {Z0+pen*PEN_DZ+1+i*0}.{1+i*3} {{VillagerData:{{type:"minecraft:plains",profession:"minecraft:none",level:1}},PersistenceRequired:1b,Tags:["wp66"]}}' for pen in range(pens) for i in range(3)])
    run([f'summon minecraft:zombie {X0+5}.5 {F+1} {Z0+pen*PEN_DZ+1}.5 {{PersistenceRequired:1b,Tags:["wp66z"]}}' for pen in range(pens)])
    run(["data remove storage wp66:log golems", "scoreboard players set #n wp66 0"])
    gt = gametime()
    # the villagers slept just now
    run([f'execute as @e[type=minecraft:villager,tag=wp66] run data modify entity @s Brain.memories."minecraft:last_slept" set value {{value:{gt}L}}'])


if __name__ == "__main__":
    ticks, pens, outp = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    build(pens)
    run(["gamerule advance_time true"])
    g0 = gametime()
    n = advance(ticks)
    out = run(["data get storage wp66:log golems", "scoreboard players get #n wp66"], timeout=600)
    rows = g.pattern_rows(" ".join(out))
    json.dump({"target": T, "ticks": n, "pens": pens, "g_start": g0, "golems": rows}, open(outp, "w"))
    print(f"TARGET={T}: {len(rows)} golems in {n} ticks, {pens} pens; {out[-1][:60]}")
