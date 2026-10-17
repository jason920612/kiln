"""wp74: small lock-step trials on an identical y200 platform, Kiln against vanilla (not frozen: `tick sprint` rounds on both).
usage: p74_misc.py endermite [TRIALS] | turtles [PAIRS] | eggs [N]"""
import sys, threading, json
from lock import *

ME = "KilnTest"
X0, Y0, Z0 = 100, 200, 100


def prep(t):
    if t == "V":
        w(t, f"op {ME}")
    for g in ("spawn_mobs false", "mob_griefing true", "advance_time false", "advance_weather false", "random_tick_speed 3"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "time set 1000"); w(t, "weather clear")
    w(t, f"gamemode creative {ME}")
    w(t, f"forceload add {X0 - 20} {Z0 - 20} {X0 + 120} {Z0 + 120}")
    w(t, f"tp {ME} {X0 + 60} {Y0 + 25} {Z0 + 60}")
    w(t, f"effect give {ME} minecraft:slow_falling 99999 0 true")
    time.sleep(15)


def endermite(t, trials, out):
    prep(t)
    w(t, f"fill {X0} {Y0 - 1} {Z0} {X0 + 119} {Y0 + 6} {Z0 + 119} minecraft:air")
    w(t, f"fill {X0} {Y0 - 1} {Z0} {X0 + 119} {Y0 - 1} {Z0 + 119} minecraft:stone")
    w(t, "kill @e[type=!player]")
    time.sleep(1)
    cells = [(X0 + 4 + 12 * (i % 9), Z0 + 4 + 12 * (i // 9)) for i in range(trials)]
    for (x, z) in cells:
        w(t, f"summon minecraft:enderman {x}.5 {Y0} {z}.5 {{PersistenceRequired:1b}}")
        w(t, f"summon minecraft:endermite {x + 5}.5 {Y0} {z}.5 {{PersistenceRequired:1b}}")
    res = []
    for r in range(8):
        sprint(t, 50)
        res.append(count(t, "@e[type=endermite]"))
    out[t] = {"endermites_left_every_50_ticks": res, "endermen": count(t, "@e[type=enderman]")}


def turtles(t, pairs, out):
    prep(t)
    w(t, f"fill {X0} {Y0 - 6} {Z0} {X0 + 119} {Y0 + 6} {Z0 + 119} minecraft:air")
    w(t, f"fill {X0} {Y0 - 5} {Z0} {X0 + 119} {Y0 - 1} {Z0 + 119} minecraft:stone")
    cells = [(X0 + 5 + 18 * (i % 6), Z0 + 5 + 18 * (i // 6)) for i in range(pairs)]
    for (x, z) in cells:
        # a 7x7 sand beach (y Y0-1) with a 5x5 pond (depth 2) cut in the middle, the turtles start in the pond
        w(t, f"fill {x - 5} {Y0 - 1} {z - 5} {x + 5} {Y0 - 1} {z + 5} minecraft:sand")
        w(t, f"fill {x - 1} {Y0 - 3} {z - 1} {x + 1} {Y0 - 1} {z + 1} minecraft:water")
    w(t, "kill @e[type=!player]")
    time.sleep(1)
    for (x, z) in cells:
        for dx in (0, 1):
            w(t, f"summon minecraft:turtle {x + dx - 0}.5 {Y0 - 2} {z}.5 {{PersistenceRequired:1b,InLove:2400,Age:0,home_pos:[I;{x},{Y0 - 1},{z}]}}")
    laid = []
    t_first = {}
    for r in range(30):
        sprint(t, 100)
        n = 0
        for (x, z) in cells:
            n += count(t, f"@e[type=turtle,x={x - 6},y={Y0 - 4},z={z - 6},dx=12,dy=8,dz=12,nbt={{has_egg:1b}}]")
        eggs = 0
        for (x, z) in cells:
            o = ask(t, f"fill {x - 6} {Y0 - 1} {z - 6} {x + 6} {Y0 + 1} {z + 6} minecraft:air replace minecraft:turtle_egg", wait=0.15)
            m = re.search(r"success\[(\d+)\]|Successfully filled (\d+)|Changed (\d+)", o)
            eggs += int(next(g for g in m.groups() if g)) if m else 0
        laid.append((100 * (r + 1), n, eggs))
    out[t] = {"(ticks, turtles carrying an egg, egg blocks laid since last look)": laid,
              "babies": count(t, "@e[type=turtle,nbt={Age:-24000}]") if False else 0, "adults": count(t, "@e[type=turtle]")}


MODES = {"endermite": endermite, "turtles": turtles}
if __name__ == "__main__":
    mode, n = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 30
    out = {}
    th = [threading.Thread(target=MODES[mode], args=(t, n, out)) for t in "KV"]
    [x.start() for x in th]; [x.join() for x in th]
    print("RESULT", json.dumps(out))
