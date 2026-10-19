"""wp81 item 3 (zombified piglins on the nether roof): the fill curve of the monster cap. Same world and farm as farm81.py's `nether` scenario (seed 77, a 15x15 stone platform on the
roof, the player 60 blocks above it). Per round: kill every non-player entity, then `tick step 5` and count the entities of the level, ROUNDS rounds; the mean count after every
step on Kiln (K) and vanilla (V), the ticks until the count passes 30 / 60, and the count of the platform's zombified piglins at the end of each round.
usage: fill81.py [ROUNDS] [STEPS]   (STEPS of 5 ticks per round, default 24 = 120 ticks)"""
import re, sys, time
from lk import *

ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 12
STEPS = int(sys.argv[2]) if len(sys.argv) > 2 else 24
ME = "W0"
x0, z0 = -64, 64


def cmd(t, c, wait=0.15):
    return ask(t, c, wait=wait)


def prep(t):
    if t == "V":
        w(t, f"op {ME}")
    for g in ("spawn_mobs true", "spawn_monsters true", "advance_time false", "advance_weather false", "spawn_phantoms false", "mob_griefing false",
              "random_tick_speed 3", "spawn_wardens false", "command_block_output false", "log_admin_commands false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal")
    w(t, "weather clear")
    w(t, "time set 18000")
    w(t, f"gamemode creative {ME}")
    w(t, f"effect give {ME} minecraft:resistance 99999 4 true")
    time.sleep(0.5)
    w(t, f"execute in minecraft:the_nether run tp {ME} {x0 + 0.5} 191 {z0 + 0.5}")
    time.sleep(1.0)
    cmd(t, f"execute in minecraft:the_nether run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    time.sleep(25)
    cmd(t, f"execute in minecraft:the_nether run fill {x0 - 2} 189 {z0 - 2} {x0 + 2} 190 {z0 + 2} minecraft:glass")
    w(t, f"execute in minecraft:the_nether run tp {ME} {x0 + 0.5} 191 {z0 + 0.5}")
    time.sleep(1.0)
    cmd(t, f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 133 {z0 + 7} minecraft:air")
    cmd(t, f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 128 {z0 + 7} minecraft:stone")
    for a, b, c, d in ((-8, -8, 8, -8), (-8, 8, 8, 8), (-8, -8, -8, 8), (8, -8, 8, 8)):
        cmd(t, f"execute in minecraft:the_nether run fill {x0 + a} 129 {z0 + b} {x0 + c} 132 {z0 + d} minecraft:stone")
    cmd(t, f"execute in minecraft:the_nether run fill {x0 - 8} 133 {z0 - 8} {x0 + 8} 133 {z0 + 8} minecraft:stone")


def count(t, sel):
    out = ask(t, f"execute in minecraft:the_nether if entity {sel}", wait=0.1)
    m = re.search(r"count: (\d+)|pass_count\[(\d+)\]", out)
    if m:
        return int(m.group(1) or m.group(2))
    return 1 if ("passed" in out or "conditional.pass" in out) else 0


ALL = "@e[type=!player,x=-100000,y=-64,z=-100000,dx=200000,dy=500,dz=200000]"
BOX = f"@e[type=minecraft:zombified_piglin,x={x0 - 7},y=129,z={z0 - 7},dx=14,dy=3,dz=14]"
for t in "KV":
    prep(t)
both("tick freeze")
time.sleep(0.5)
curve = {t: [[] for _ in range(STEPS)] for t in "KV"}
reach = {t: {30: [], 60: []} for t in "KV"}
zp = {t: [] for t in "KV"}
for r in range(ROUNDS):
    for t in "KV":
        w(t, "kill @e[type=!player]")
    time.sleep(0.4)
    seen = {t: {30: None, 60: None} for t in "KV"}
    for k in range(STEPS):
        step(5)
        for t in "KV":
            n = count(t, ALL)
            curve[t][k].append(n)
            for lim in (30, 60):
                if seen[t][lim] is None and n >= lim:
                    seen[t][lim] = (k + 1) * 5
    for t in "KV":
        for lim in (30, 60):
            reach[t][lim].append(seen[t][lim])
        zp[t].append(count(t, BOX))
    print("round", r, "reach 30/60: K", seen["K"][30], seen["K"][60], " V", seen["V"][30], seen["V"][60], " box zombified piglins K", zp["K"][-1], "V", zp["V"][-1])
    sys.stdout.flush()
print("mean entity count after 5*(k+1) ticks:")
for k in range(0, STEPS):
    print(" ", (k + 1) * 5, "K %.1f" % (sum(curve["K"][k]) / ROUNDS), " V %.1f" % (sum(curve["V"][k]) / ROUNDS))
for t in "KV":
    for lim in (30, 60):
        v = [x for x in reach[t][lim] if x is not None]
        print(t, "ticks until", lim, "entities: mean", (sum(v) / len(v)) if v else None, "of", len(v), "rounds")
    print(t, "zombified piglins on the platform at the end of the rounds, total", sum(zp[t]))
