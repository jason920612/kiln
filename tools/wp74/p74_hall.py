"""wp74: a trading hall on identical platforms, Kiln against vanilla (lock-step not needed: `tick sprint` rounds on both).
N cells (1x2, a workstation at the far end) each get one jobless villager: how long until they all have the job of their station (claim),
then every villager gets sold-out offers (uses = 99) and over TWO days we record how many of them restocked and RestocksToday.
Part C ("steal"): 6 villagers in one open room with 3 workstations: who ends up employed.
usage: p74_hall.py [N] [DAYS]"""
import sys, threading, json
from lock import *

N = int(sys.argv[1]) if len(sys.argv) > 1 else 12
DAYS = int(sys.argv[2]) if len(sys.argv) > 2 else 2
ME = "KilnTest"
X0, Y0, Z0 = 100, 200, 100
STATIONS = ["lectern", "composter", "blast_furnace", "smoker", "fletching_table", "loom", "grindstone", "cartography_table", "brewing_stand", "barrel", "smithing_table", "stonecutter"]
PROF = {"lectern": "librarian", "composter": "farmer", "blast_furnace": "armorer", "smoker": "butcher", "fletching_table": "fletcher", "loom": "shepherd",
        "grindstone": "weaponsmith", "cartography_table": "cartographer", "brewing_stand": "cleric", "barrel": "fisherman", "smithing_table": "toolsmith", "stonecutter": "mason"}


def go(t, out):
    if t == "V":
        w(t, f"op {ME}")
    for g in ("spawn_mobs false", "advance_weather false", "random_tick_speed 3", "mob_griefing false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "weather clear"); w(t, f"gamemode creative {ME}")
    w(t, f"forceload add {X0 - 4} {Z0 - 4} {X0 + 60} {Z0 + 30}")
    w(t, f"execute in minecraft:overworld run tp {ME} {X0 + 20} {Y0 + 12} {Z0 + 10}")
    w(t, f"effect give {ME} minecraft:slow_falling 99999 0 true")
    wait_loaded(t, X0 + 60, Y0, Z0 + 30)
    wait_loaded(t, X0 - 2, Y0, Z0 - 2)
    time.sleep(2)
    w(t, f"fill {X0 - 2} {Y0 - 1} {Z0 - 2} {X0 + 60} {Y0 + 6} {Z0 + 30} minecraft:air")
    w(t, f"fill {X0 - 2} {Y0 - 1} {Z0 - 2} {X0 + 60} {Y0 - 1} {Z0 + 30} minecraft:stone")
    w(t, "kill @e[type=!player]")
    w(t, "time set 0")
    st = {}
    for i in range(N):
        x, z = X0 + 3 * i, Z0
        w(t, f"fill {x - 1} {Y0} {z - 1} {x + 1} {Y0 + 2} {z + 3} minecraft:stone")
        w(t, f"fill {x} {Y0} {z} {x} {Y0 + 1} {z + 1} minecraft:air")
        w(t, f"fill {x} {Y0 + 2} {z} {x} {Y0 + 2} {z + 1} minecraft:glass")
        s = STATIONS[i % len(STATIONS)]
        w(t, f"setblock {x} {Y0} {z + 1} minecraft:{s}")
        w(t, f"summon minecraft:villager {x}.5 {Y0} {z}.5 {{PersistenceRequired:1b,Tags:[\"hall{i}\"],VillagerData:{{profession:\"minecraft:none\",level:1,type:\"minecraft:plains\"}}}}")
    time.sleep(1)
    rows = []
    employed_at = None
    for r in range(30):
        sprint(t, 40)
        emp = 0
        for i in range(N):
            p = PROF[STATIONS[i % len(STATIONS)]]
            emp += count(t, f'@e[type=villager,tag=hall{i},nbt={{VillagerData:{{profession:"minecraft:{p}"}}}}]')
        rows.append((40 * (r + 1), emp))
        if emp == N:
            break
    out[t + "_claim"] = rows

    # sold-out offers
    w(t, "time set 1000")
    w(t, 'execute as @e[type=villager] run data merge entity @s {Offers:{Recipes:[{buy:{id:"minecraft:emerald",count:1},sell:{id:"minecraft:bread",count:1},uses:99,maxUses:12,xp:1,priceMultiplier:0.05f,demand:0,specialPrice:0,rewardExp:1b}]}}')
    time.sleep(0.5)
    base = count(t, "@e[type=villager,nbt={Offers:{Recipes:[{uses:99}]}}]")
    rows = []
    for k in range(DAYS * 24):
        sprint(t, 1000)
        sold = count(t, "@e[type=villager,nbt={Offers:{Recipes:[{uses:99}]}}]")
        rs = [count(t, f"@e[type=villager,nbt={{RestocksToday:{v}}}]") for v in (0, 1, 2)]
        rows.append((1000 * (k + 1), sold, rs))
    out[t + "_restock"] = {"villagers still sold out at the start": base, "(tick, still sold out, RestocksToday=0/1/2 counts)": rows}

    # part C: stealing. A 12x12 room, 3 workstations and 6 jobless villagers: after 3000 ticks how many are employed, which job sites are taken twice
    sx, sz = X0 + 20, Z0 + 12
    trials = []
    for trial in range(6):
        w(t, "kill @e[type=villager]")
        w(t, f"fill {sx - 1} {Y0} {sz - 1} {sx + 12} {Y0 + 4} {sz + 12} minecraft:stone hollow")
        w(t, f"fill {sx} {Y0} {sz} {sx + 11} {Y0 + 3} {sz + 11} minecraft:air")
        for k, (dx, dz, blk) in enumerate(((1, 1, "lectern"), (10, 1, "composter"), (5, 10, "barrel"))):
            w(t, f"setblock {sx + dx} {Y0} {sz + dz} minecraft:{blk}")
        time.sleep(0.5)
        for k in range(6):
            w(t, f"summon minecraft:villager {sx + 2 + 2 * (k % 4)}.5 {Y0} {sz + 4 + 3 * (k // 4)}.5 {{PersistenceRequired:1b,Tags:[\"steal\"],VillagerData:{{profession:\"minecraft:none\",level:1,type:\"minecraft:plains\"}}}}")
        row = []
        for r in range(6):
            sprint(t, 500)
            row.append(count(t, '@e[type=villager,tag=steal,nbt=!{VillagerData:{profession:"minecraft:none"}}]'))
        trials.append(row)
    out[t + "_steal"] = trials

if __name__ == "__main__":
    out = {}
    th = [threading.Thread(target=go, args=(t, out)) for t in "KV"]
    [x.start() for x in th]; [x.join() for x in th]
    print("RESULT", json.dumps(out))
