"""wp72: ocean odds and ends, Kiln against vanilla (one process, both consoles): (1) drowned equipment (trident / rod / nautilus shell shares of
N summoned drowned), (2) turtle eggs: hatch stages on sand at night and trampling by a zombie, (3) a conduit's frame lit up and broken.
usage: p72_ocean.py [drowned|eggs] [N]"""
import re, sys
from lock import *

X0, Y0, Z0 = 12, 200, 12


def count_sel(t, sel):
    o = ask(t, f"execute if entity {sel}", wait=0.15)
    return o


def drowned(n):
    setup(frozen=False)
    both_("gamemode creative KilnTest")
    both_("tp KilnTest 28 205 14")
    both_("fill 10 199 10 40 205 40 minecraft:air"); both_("fill 10 199 10 40 199 40 minecraft:stone")
    both_("kill @e[type=!player]")
    time.sleep(1.5)
    res = {"K": {"trident": 0, "fishing_rod": 0, "nautilus_shell": 0}, "V": {"trident": 0, "fishing_rod": 0, "nautilus_shell": 0}}
    for i in range(n):
        both_(f'summon minecraft:drowned {12 + (i % 28)}.5 200 {12 + (i // 28 % 28)}.5')
        if i % 25 == 24:
            time.sleep(0.8)
    time.sleep(3)
    for t in "KV":
        for item, sel in (("trident", "mainhand"), ("fishing_rod", "mainhand"), ("nautilus_shell", "offhand")):
            slot = "weapon.mainhand" if sel == "mainhand" else "weapon.offhand"
            o = ask(t, f"execute as @e[type=minecraft:drowned] if items entity @s {slot} minecraft:{item} run say HAS", wait=1.2)
            res[t][item] = o.count("HAS")
    total = {t: ask(t, 'execute as @e[type=minecraft:drowned] run say DR', wait=1.2).count("DR") for t in "KV"}
    print("drowned summoned:", n, "alive tagged:", total)
    print("K:", res["K"], "\nV:", res["V"])
    both_("kill @e[type=!player]")


def eggs():
    setup(rules=("random_tick_speed 300",))
    both_("gamemode creative KilnTest"); both_("tp KilnTest 28 205 14"); both_("time set 18000")
    both_(f"fill 9 199 9 40 205 40 minecraft:air"); both_("fill 9 199 9 40 199 40 minecraft:sand")
    both_("kill @e[type=!player]")
    both_("tick step 20"); time.sleep(2)
    for i in range(8):
        both_(f"setblock {12 + 2 * i} 200 12 minecraft:turtle_egg[eggs=4,hatch=0]")
    rows = []
    for k in range(40):
        step(100)
        out = {}
        for t in "KV":
            o = ask(t, "execute as @e[type=minecraft:turtle] run say T", wait=0.3)
            hatch = []
            for i in range(8):
                b = ask(t, f"execute if block {12 + 2 * i} 200 12 minecraft:turtle_egg[hatch=2] run say H2", wait=0.1)
                hatch.append(b.count("H2"))
            eggs_left = sum(ask(t, f"execute if block {12 + 2 * i} 200 12 minecraft:turtle_egg run say E", wait=0.1).count("E") for i in range(8))
            out[t] = (o.count("T"), eggs_left)
        rows.append((100 * (k + 1), out["K"], out["V"]))
        print("tick %5d  turtles/eggs-blocks K %s  V %s" % rows[-1], flush=True)
        if out["K"][1] == 0 and out["V"][1] == 0:
            break
    both_("tick unfreeze")


if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "drowned"
    if what == "drowned":
        drowned(int(sys.argv[2]) if len(sys.argv) > 2 else 400)
    else:
        eggs()
