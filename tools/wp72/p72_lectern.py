"""wp72: librarian rerolling. N glass cells with one unemployed, never-traded villager each (named c0..); a lectern is placed, the
villager claims it (profession librarian), the lectern is broken (a villager without xp and above level 1 loses the profession), placed again...
Times (in ticks) of each step are compared between Kiln and vanilla. usage: p72_lectern.py [CELLS] [CYCLES]"""
import re, sys
from lock import *

CELLS = int(sys.argv[1]) if len(sys.argv) > 1 else 6
CYCLES = int(sys.argv[2]) if len(sys.argv) > 2 else 3
Y = 200
STEP = 10
LECT = "minecraft:lectern[facing=north]"


def cell(i):
    return 12 + 6 * i, 12


def who(t, prof):
    out = ask(t, f'execute as @e[type=minecraft:villager,nbt={{VillagerData:{{profession:"minecraft:{prof}"}}}}] run say @s', wait=0.25)
    return sorted(set(int(m) for m in re.findall(r"\bc(\d)\b", out)))


def build():
    for i in range(CELLS):
        x, z = cell(i)
        both_(f"fill {x - 3} {Y - 1} {z - 3} {x + 3} {Y + 4} {z + 3} minecraft:air")
        both_(f"fill {x - 2} {Y - 1} {z - 2} {x + 2} {Y + 3} {z + 2} minecraft:glass outline")
        both_(f"fill {x - 2} {Y - 1} {z - 2} {x + 2} {Y - 1} {z + 2} minecraft:stone")
        both_(f'summon minecraft:villager {x + 0.5} {Y} {z + 0.5} {{CustomName:"c{i}",PersistenceRequired:1b,VillagerData:{{profession:"minecraft:none",level:1,type:"minecraft:plains"}},Xp:0}}')


def lectern_at(i):
    x, z = cell(i)
    return x + 1, Y, z - 1


def main():
    setup()
    both_("gamemode creative KilnTest")
    both_("tp KilnTest 40 205 40")
    both_("kill @e[type=!player]")
    both_("tick step 20")
    time.sleep(2)
    build()
    both_("tick step 40")
    time.sleep(3)
    results = {"K": [], "V": []}
    for cyc in range(CYCLES):
        for kind, want_prof, place in (("claim", "librarian", True), ("lose", "none", False)):
            for i in range(CELLS):
                x, y, z = lectern_at(i)
                both_(f"setblock {x} {y} {z} {LECT if place else 'minecraft:air'}")
            elapsed = 0
            done = {"K": {}, "V": {}}
            while elapsed < 400 and any(len(done[t]) < CELLS for t in "KV"):
                step(STEP)
                elapsed += STEP
                for t in "KV":
                    for i in who(t, want_prof):
                        done[t].setdefault(i, elapsed)
            for t in "KV":
                results[t].append((cyc, kind, [done[t].get(i) for i in range(CELLS)]))
            print(f"cycle {cyc} {kind}:")
            for t in "KV":
                print("  ", t, results[t][-1][2], flush=True)
    for t in "KV":
        for kind in ("claim", "lose"):
            xs = [v for (c, k, row) in results[t] if k == kind for v in row if v is not None]
            miss = sum(1 for (c, k, row) in results[t] if k == kind for v in row if v is None)
            print(f"{t} {kind}: n={len(xs)} mean={sum(xs) / max(1, len(xs)):.0f} min={min(xs) if xs else None} max={max(xs) if xs else None} missing={miss}")
    both_("tick unfreeze")


main()
