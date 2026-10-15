"""wp66 item 1: an iron golem farm that works in vanilla 26.3 and the same farm on Kiln (console only).
Three villagers sleep in beds in a pen (nights are run for real), a zombie they can see stands behind a fence; `VillagerPanicTrigger`
spawns golems (every 100 game ticks while a villager panics, if 3 villagers slept within a day and none saw a golem lately).
A datapack tick function (wp66:tick) logs each new golem (time and position) in storage wp66:log and kills it.
usage: TARGET=K|V rt_golem.py setup | run TICKS | report"""
import sys, re, json
from p66rt import *

X0 = int(os.environ.get("GX", "4000")); Z0 = int(os.environ.get("GZ", "4000")); F = -61
# pen interior x 0..4, z 0..4 (relative), zombie cell x 6..7; the fence line at x = 5


def at(dx, dy, dz):
    return f"{X0+dx} {F+dy} {Z0+dz}"


def setup(villagers=3, zombie=True, difficulty="normal"):
    run([f"forceload add {X0-12} {Z0-12} {X0+20} {Z0+16}"])
    assert ready(X0, Z0) and ready(X0 + 20, Z0 + 16) and ready(X0 - 12, Z0 - 12)
    run(["tick freeze", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "gamerule mob_griefing true",
         f"difficulty {difficulty}", "weather clear 1000000", "time set 11500", "kill @e[type=!player]"], timeout=300)
    cmds = []
    # ground (a Kiln world without a generator is void), a stone platform for the golems around the pen
    cmds.append(f"fill {X0-12} {F} {Z0-12} {X0+20} {F} {Z0+16} minecraft:stone")
    cmds.append(f"fill {X0-12} {F+1} {Z0-12} {X0+20} {F+8} {Z0+16} minecraft:air")
    # pen: walls x -1, z -1 and 5, roof at y+3; zombie cell x 6..7 with its own wall x 8
    cmds.append(f"fill {X0-1} {F+1} {Z0-1} {X0+8} {F+3} {Z0+5} minecraft:stone_bricks")
    cmds.append(f"fill {X0} {F+1} {Z0} {X0+4} {F+2} {Z0+4} minecraft:air")
    cmds.append(f"fill {X0+6} {F+1} {Z0} {X0+7} {F+2} {Z0+4} minecraft:air")
    for z in range(5):
        cmds.append(f"setblock {at(5, 1, z)} minecraft:oak_fence")
        cmds.append(f"setblock {at(5, 2, z)} minecraft:air")
    # beds along the west wall (head west), three of them
    for z in (0, 2, 4):
        cmds.append(f"setblock {at(0, 1, z)} minecraft:red_bed[facing=east,part=head]")
        cmds.append(f"setblock {at(1, 1, z)} minecraft:red_bed[facing=east,part=foot]")
    # light, so nothing about darkness differs
    cmds.append(f"setblock {at(3, 2, 0)} minecraft:air")
    run(cmds, timeout=3600)
    run([f"setblock {at(0,0,0)} minecraft:stone"])


def populate(villagers=3, zombie=True):
    cmds = []
    for i in range(villagers):
        cmds.append(f'summon minecraft:villager {X0+3}.5 {F+1} {Z0+1+i}.5 {{VillagerData:{{type:"minecraft:plains",profession:"minecraft:none",level:1}},PersistenceRequired:1b,Tags:["wp66"]}}')
    if zombie:
        cmds.append(f'summon minecraft:zombie {X0+6}.5 {F+1} {Z0+2}.5 {{PersistenceRequired:1b,Tags:["wp66"],IsBaby:0b}}')
    run(cmds)


def villager_states():
    out = run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Brain.memories"])
    return out


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "setup":
        setup(); populate()
        print("ok")
    elif cmd == "mem":
        for l in villager_states(): print(l[:600])
    elif cmd == "build":
        setup(); populate(zombie=False)
        print("ok")
    elif cmd == "zombie":
        run([f'summon minecraft:zombie {X0+6}.5 {F+1} {Z0+2}.5 {{PersistenceRequired:1b,Tags:["wp66"]}}'])
    elif cmd == "morning":
        dt = day_time()
        n = (24000 - dt + int(sys.argv[2] if len(sys.argv) > 2 else 300)) % 24000
        run(["gamerule advance_time true"])
        print("day", dt, "-> advancing", n, advance(n), "now", day_time())
    elif cmd == "golems":
        for l in run(["execute as @e[type=minecraft:iron_golem] run data get entity @s Pos"]): print(l[:200])
        for l in run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Brain.memories"]): print(l[:400])
    elif cmd == "go":
        run(["gamerule advance_time true"])
        n = advance(int(sys.argv[2]))
        print("advanced", n, "time", run(["time query daytime"]))
        for l in run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Pos", "execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Brain.memories", "execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Sleeping"]):
            print(l[:700])
