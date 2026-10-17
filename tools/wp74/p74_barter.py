"""wp74: piglin bartering, Kiln against vanilla, same setup in the nether (a sealed 20x20 room at y 70). N piglins each get one gold ingot dropped
at their feet per round (they pick it up, admire it for 120 ticks and throw the loot); the loot items are counted by id after every round.
usage: p74_barter.py [PIGLINS] [ROUNDS]"""
import sys, threading, json, collections
from lock import *

NP = int(sys.argv[1]) if len(sys.argv) > 1 else 40
ROUNDS = int(sys.argv[2]) if len(sys.argv) > 2 else 20
IDS = ["enchanted_book", "iron_boots", "iron_nugget", "splash_potion", "potion", "quartz", "glowstone_dust", "magma_cream", "ender_pearl", "string",
       "fire_charge", "gravel", "leather", "nether_brick", "obsidian", "crying_obsidian", "soul_sand", "spectral_arrow", "water_bottle", "gold_ingot"]
X0, Y0, Z0 = 400, 70, 400
NETHER = "execute in minecraft:the_nether run "


def go(t, out):
    def c(cmd, wait=0.1):
        return ask(t, NETHER + cmd, wait=wait)
    if t == "V":
        w(t, "op KilnTest")
    w(t, "gamerule spawn_mobs false")
    w(t, "gamerule mob_griefing true")
    w(t, "gamemode creative KilnTest")
    w(t, "difficulty normal")
    c(f"forceload add {X0 - 8} {Z0 - 8} {X0 + 30} {Z0 + 30}")
    w(t, NETHER + f"tp KilnTest {X0 + 10} {Y0 + 1} {Z0 + 10}")
    time.sleep(12)
    c(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + 20} {Y0 + 4} {Z0 + 20} minecraft:air")
    c(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + 20} {Y0 + 4} {Z0 + 20} minecraft:stone hollow")
    c(f"fill {X0} {Y0 - 1} {Z0} {X0 + 19} {Y0 - 1} {Z0 + 19} minecraft:netherrack")
    c("kill @e[type=!player]")
    pos = [(X0 + 1.5 + (i % 18), Y0, Z0 + 1.5 + (i // 18) * 2) for i in range(NP)]
    for (x, y, z) in pos:
        c(f"summon minecraft:piglin {x} {y} {z} {{PersistenceRequired:1b,IsImmuneToZombification:1b,Brain:{{}}}}", wait=0.02)
    time.sleep(1)
    n_pig = kill_count_nether(t, "@e[type=minecraft:piglin]", count_only=True)
    totals = collections.Counter()
    for r in range(ROUNDS):
        for (x, y, z) in pos:
            c(f'summon minecraft:item {x} {y} {z} {{Item:{{id:"minecraft:gold_ingot",count:1}},Age:0s,PickupDelay:0s}}', wait=0.02)
        sprint(t, 200)
        time.sleep(0.5)
        for i in IDS:
            n = kill_count(t, NETHER + f'@e[type=item,nbt={{Item:{{id:"minecraft:{i}"}}}}]') if False else kill_count(t, f'@e[type=item,nbt={{Item:{{id:"minecraft:{i}"}}}}]')
            totals[i] += n
        rest = kill_count(t, "@e[type=item]")
        totals["(other)"] += rest
        print(t, "round", r, dict((k, v) for k, v in totals.items() if v), flush=True)
    out[t] = dict(totals)
    out[t + "_piglins"] = n_pig


def kill_count_nether(t, sel, count_only=False):
    o = ask(t, NETHER + f"execute if entity {sel}", wait=0.3)
    m = re.search(r"pass_count\[(\d+)\]|Count: (\d+)", o)
    return int(m.group(1) or m.group(2)) if m else 0


out = {}
th = [threading.Thread(target=go, args=(t, out)) for t in "KV"]
[x.start() for x in th]; [x.join() for x in th]
print("RESULT", json.dumps(out))
