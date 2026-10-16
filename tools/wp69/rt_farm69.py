"""wp69 (from wp66 rt_golem.py; env ALIGN=1 makes the day time agree with the game time modulo 100, so that the 100-tick opportunities of the panic trigger fall on whole hundreds of the day):
wp66 item 1: an iron golem farm that works in vanilla 26.3 and the same farm on Kiln (console only, `tick sprint`).
Each pen: three villagers sleep in beds (nights are run for real, the zombie is away), then in the daytime a zombie they can see stands
behind a fence in the next cell; `VillagerPanicTrigger` spawns a golem every time (every 100 game ticks while a villager panics) three
villagers slept within the last day and none saw a golem in the last 600 ticks. The datapack tick function wp66:tick logs each new golem
(game time and position) in storage wp66:log and removes it (so the next one can come as soon as the villagers allow).
usage: TARGET=K|V rt_golem.py run DAYS PENS OUT.json"""
import sys, re, json, os
from p69rt import *

X0 = int(os.environ.get("GX", "4000")); Z0 = int(os.environ.get("GZ", "4000")); F = -61
PEN_DZ = 40


def at(pen, dx, dy, dz):
    return f"{X0+dx} {F+dy} {Z0+pen*PEN_DZ+dz}"


def build(pens):
    run([f"forceload add {X0-12} {Z0-12} {X0+24} {Z0+pens*PEN_DZ+20}"])
    assert ready(X0, Z0) and ready(X0 + 24, Z0 + pens * PEN_DZ + 20) and ready(X0 - 12, Z0 - 12)
    run(["tick freeze", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "gamerule mob_drops false",
         "difficulty normal", "weather clear 1000000", "time set 11500", "kill @e[type=!player]", "scoreboard objectives add wp66 dummy"], timeout=300)
    for pen in range(pens):
        cmds = [
            f"fill {X0-12} {F} {Z0+pen*PEN_DZ-12} {X0+24} {F} {Z0+pen*PEN_DZ+20} minecraft:stone",
            f"fill {X0-12} {F+1} {Z0+pen*PEN_DZ-12} {X0+24} {F+8} {Z0+pen*PEN_DZ+20} minecraft:air",
            f"fill {at(pen,-1,1,-1)} {at(pen,12,3,9)} minecraft:stone_bricks",
            f"fill {at(pen,0,1,0)} {at(pen,8,2,8)} minecraft:air",
            f"fill {at(pen,10,1,0)} {at(pen,11,2,8)} minecraft:air",
        ]
        # (the beds stand well away from the walls: a villager waking up may be put two blocks from its bed, through a wall)
        for z in range(9):
            cmds.append(f"setblock {at(pen,9,1,z)} minecraft:oak_fence")
            cmds.append(f"setblock {at(pen,9,2,z)} minecraft:air")
        for z in (2, 4, 6):
            cmds.append(f"setblock {at(pen,3,1,z)} minecraft:red_bed[facing=east,part=head]")
            cmds.append(f"setblock {at(pen,4,1,z)} minecraft:red_bed[facing=east,part=foot]")
        run(cmds, timeout=3600)
    cmds = []
    for pen in range(pens):
        for i in range(3):
            cmds.append(f'summon minecraft:villager {X0+6}.5 {F+1} {Z0+pen*PEN_DZ+3+i*2}.5 {{VillagerData:{{type:"minecraft:plains",profession:"minecraft:none",level:1}},PersistenceRequired:1b,Tags:["wp66"]}}')
    run(cmds)
    run(["data remove storage wp66:log golems", "scoreboard players set #n wp66 0"])


def zombies(pens, on):
    if not on:
        run(["kill @e[type=minecraft:zombie,tag=wp66z]"])
        return
    run([f'summon minecraft:zombie {X0+10}.5 {F+1} {Z0+pen*PEN_DZ+4}.5 {{PersistenceRequired:1b,Tags:["wp66z"]}}' for pen in range(pens)])


def state(pens):
    out = run(["execute as @e[type=minecraft:villager,tag=wp66] run data get entity @s Brain.memories"])
    slept = sum(1 for l in out if "last_slept" in l)
    detected = sum(1 for l in out if "golem_detected_recently" in l)
    return slept, detected, len(out)


def golem_log():
    out = run(["data get storage wp66:log golems", "scoreboard players get #n wp66"])
    txt = " ".join(out)
    rows = [(int(t), int(x) / 1000, int(y) / 1000, int(z) / 1000) for t, x, y, z in re.findall(r"\{t: (-?\d+), x: (-?\d+), y: (-?\d+), z: (-?\d+)\}", re.sub(r"(\w): ", r"\1: ", txt.replace("y: ", "y: ").replace("x: ", "x: ")) )] if txt else []
    return rows, out


def pattern_rows(txt):
    rows = []
    # entries look like {t: 12L?...} depending on the type; accept any order of keys
    for m in re.finditer(r"\{([^{}]*)\}", txt):
        d = dict(re.findall(r"(\w+): (-?\d+)", m.group(1)))
        if {"t", "x", "y", "z"} <= d.keys():
            rows.append((int(d["t"]), int(d["x"]) / 1000, int(d["y"]) / 1000, int(d["z"]) / 1000))
    return rows


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "run":
        days, pens, outp = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
        build(pens)
        if os.environ.get("ALIGN"):
            gt = gametime()
            run([f"time set {11500 + gt % 100}"])
            assert (day_time() - gametime()) % 100 == 0, (day_time(), gametime())
        run(["gamerule advance_time true"])
        t0 = time.time()
        g_start = gametime()
        timeline = []
        # the first night: they go to bed (time 11500 .. next morning)
        for day in range(days + 1):
            dt = day_time()
            if dt >= 11000 or day == 0:
                n = (24000 - dt + 300) % 24000
                zombies(pens, False)
                advance(n)
                s = state(pens)
                timeline.append(("morning", day, gametime(), s))
                print(f"day {day}: morning after the night: villagers slept/detected/total {s}", flush=True)
            if day == days:
                break
            zombies(pens, True)
            dt = day_time()
            advance(11000 - dt)
            timeline.append(("dusk", day, gametime(), state(pens)))
            run([f"say wp66 day {day} done"])
        out = run(["data get storage wp66:log golems", "scoreboard players get #n wp66"], timeout=600)
        rows = pattern_rows(" ".join(out))
        res = {"target": T, "days": days, "pens": pens, "g_start": g_start, "g_end": gametime(), "golems": rows, "timeline": timeline, "raw_tail": out[-1][:200]}
        json.dump(res, open(outp, "w"))
        print(f"TARGET={T}: {len(rows)} golems in {res['g_end']-g_start} ticks ({days} days, {pens} pens) [{time.time()-t0:.0f}s]; count score: {out[-1][:80]}")
