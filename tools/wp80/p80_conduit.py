"""wp80 item 1: conduit with N frame blocks (15, 16, 29, 41, 42): block entity data, conduit power on the survival player at several distances (range = N/7*16), damage to a hostile mob.
Lock step, both real players; console only."""
from lab80 import *
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 30, 160, 30
revive()
for t in "KV":
    w(t, "gamemode survival KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:water_breathing 999 0 true")
cross = []
for dx in range(-2, 3):
    for dy in range(-2, 3):
        for dz in range(-2, 3):
            ax, ay, az = abs(dx), abs(dy), abs(dz)
            if (ax > 1 or ay > 1 or az > 1) and ((dx == 0 and (ay == 2 or az == 2)) or (dy == 0 and (ax == 2 or az == 2)) or (dz == 0 and (ax == 2 or ay == 2))):
                cross.append((dx, dy, dz))
assert len(cross) == 42, len(cross)
counts = [int(a) for a in sys.argv[1].split(",")] if len(sys.argv) > 1 else [15, 16, 29, 41, 42]
for n in counts:
    both_("kill @e[type=!player]")
    both_("tick unfreeze")
    OB(f"fill {X - 20} {Y - 4} {Z - 20} {X + 20} {Y + 8} {Z + 20} minecraft:water")
    # (a big flooded box; the player's feet in it too)
    OB(f"fill {X - 20} {Y - 5} {Z - 20} {X + 20} {Y - 5} {Z + 20} minecraft:stone")
    OB(f"setblock {X} {Y} {Z} minecraft:conduit")
    for (dx, dy, dz) in cross[:n]:
        OB(f"setblock {X + dx} {Y + dy} {Z + dz} minecraft:prismarine")
    OB(f"tp KilnTest {X + 6}.5 {Y} {Z + 6}.5")
    both_("effect clear KilnTest")
    both_("tick freeze")
    time.sleep(1)
    OB(f"summon minecraft:zombie {X - 5}.5 {Y} {Z}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"z\"]}}")
    OB(f"summon minecraft:cow {X + 3}.5 {Y} {Z}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"c\"]}}")
    OB(f"summon minecraft:zombie {X + 15}.5 {Y} {Z}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"zfar\"]}}")
    res = {}
    for st in (40, 80, 160):
        step(st if st == 40 else st - (40 if st == 80 else 80))
        r = {}
        for t in "KV":
            ef = ask(t, "data get entity KilnTest active_effects", wait=0.3)
            conduit = "conduit_power" in ef
            m = re.search(r"\{[^{}]*conduit_power[^{}]*\}", ef)
            r[t] = (conduit, re.search(r"duration: (\d+)", m.group(0)).group(1) if m else None,
                    health(t, "@e[tag=z,limit=1]") if False else None)
        hz = {t: fnum(O(t, "data get entity @e[tag=z,limit=1] Health", 0.3)) for t in "KV"}
        hf = {t: fnum(O(t, "data get entity @e[tag=zfar,limit=1] Health", 0.3)) for t in "KV"}
        hc = {t: fnum(O(t, "data get entity @e[tag=c,limit=1] Health", 0.3)) for t in "KV"}
        cmpn(f"n={n:2d} t={st:3d} power(dur) / zombie5 / zombie20 / cow", (r["K"][:2], hz["K"], hf["K"], hc["K"]), (r["V"][:2], hz["V"], hf["V"], hc["V"]))
    cmpn(f"n={n:2d} conduit data", nbt("K", "block", f"{X} {Y} {Z}"), nbt("V", "block", f"{X} {Y} {Z}"))
both_("tick unfreeze")
both_("kill @e[type=!player]")
OB(f"fill {X - 20} {Y - 5} {Z - 20} {X + 20} {Y + 8} {Z + 20} minecraft:air")
