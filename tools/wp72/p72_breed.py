"""wp72: villager breeding. PAIRS glass rooms, each with 2 adult villagers holding 6 bread (willing to breed), 3 beds and a door-less
roof; time stays at day. Every SAMPLE ticks the number of baby villagers is counted on both servers. usage: p72_breed.py [PAIRS] [TICKS]"""
import re, sys
from lock import *

PAIRS = int(sys.argv[1]) if len(sys.argv) > 1 else 12
TICKS = int(sys.argv[2]) if len(sys.argv) > 2 else 6000
SAMPLE = int(os.environ.get("SAMPLE", "300"))
Y = 200


def room(i):
    return 12 + 7 * (i % 4), 12 + 7 * (i // 4)


def build():
    for i in range(PAIRS):
        x, z = room(i)
        both_(f"fill {x - 3} {Y - 1} {z - 3} {x + 3} {Y + 4} {z + 3} minecraft:air")
        both_(f"fill {x - 3} {Y - 1} {z - 3} {x + 3} {Y + 3} {z + 3} minecraft:glass outline")
        both_(f"fill {x - 3} {Y - 1} {z - 3} {x + 3} {Y - 1} {z + 3} minecraft:stone")
        for dz in (-2, 0, 2):
            both_(f"setblock {x + 2} {Y} {z + dz} minecraft:red_bed[facing=west,part=foot]")
            both_(f"setblock {x + 1} {Y} {z + dz} minecraft:red_bed[facing=west,part=head]")
        for n in "ab":
            both_(f'summon minecraft:villager {x - 1.5} {Y} {z + (0.5 if n == "a" else -0.5)} {{Tags:["parent"],PersistenceRequired:1b,VillagerData:{{profession:"minecraft:none",level:1,type:"minecraft:plains"}},Inventory:[{{id:"minecraft:bread",count:6}}]}}')


def babies(t):
    out = ask(t, "execute as @e[type=minecraft:villager,tag=!parent] run say B", wait=0.25)
    return len(re.findall(r"\bB\b", out))


def main():
    setup(rules=("do_daylight_cycle false",))
    both_("time set 1000")
    both_("gamemode creative KilnTest")
    both_("tp KilnTest 30 215 30")
    both_("kill @e[type=!player]")
    both_("tick step 20")
    time.sleep(2)
    build()
    time.sleep(1)
    rows = []
    for k in range(TICKS // SAMPLE):
        step(SAMPLE)
        rows.append((SAMPLE * (k + 1), babies("K"), babies("V")))
        print("tick %5d  babies K %2d  V %2d" % rows[-1], flush=True)
    both_("tick unfreeze")


main()
