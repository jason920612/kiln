"""wp84 #2: what a bare `/summon <type>` gives on both servers (SpawnReason COMMAND, finalizeSpawn runs): riders (jockeys), babies, equipment - N summons per type in the End
(the matrix and the play scripts clear the overworld), counted with `tag` + `kill` (the count of the kill answer). Lock-step lab (lab84): one process drives both consoles.
usage: python3 p84_jockeys.py [N] [type,type,...]"""
from lab84 import *

N = int(sys.argv[1]) if len(sys.argv) > 1 else 300
TYPES = (sys.argv[2].split(",") if len(sys.argv) > 2 else
         ("camel_husk,zombie,husk,drowned,zombie_villager,spider,cave_spider,skeleton,stray,bogged,parched,wither_skeleton,chicken,zombie_horse,skeleton_horse,horse,piglin,"
          "zombified_piglin,hoglin,zoglin,strider,nautilus,zombie_nautilus,pillager,vindicator,ravager,evoker,witch,vex,phantom,llama,trader_llama,camel,ghast,magma_cube,slime,creeper,enderman").split(","))
X0, Y, Z0 = 4000, 100, 4000
BOX = f"x={X0},y={Y},z={Z0},distance=..60"


def E(cmd):
    both_(f"execute in minecraft:overworld run {cmd}")


def kills(t, sel):
    out = ask(t, f"execute in minecraft:overworld run kill {sel}", wait=0.35)
    m = re.search(r"success\.multiple\[(\d+)\]|Killed (\d+) entities", out)
    if m:
        return int(m.group(1) or m.group(2))
    return 1 if ("success.single" in out or re.search(r"Killed [A-Za-z]", out)) else 0


def tagn(t, sel, tag):
    """tag the selector's entities, return how many."""
    out = ask(t, f"execute in minecraft:overworld run tag {sel} add {tag}", wait=0.3)
    m = re.search(r"tag\.add\.success\.multiple\[[^,\]]*, (\d+)\]|Added tag '[^']*' to (\d+) entities", out)
    if m:
        return int(next(g for g in m.groups() if g))
    return 1 if ("success.single" in out or "Added tag" in out) else 0


both_("gamerule mob_griefing false")
both_("gamerule do_daylight_cycle false")
both_("time set midnight")
both_("gamerule spawn_mobs false")
E(f"forceload add {X0 - 20} {Z0 - 20} {X0 + 20} {Z0 + 20}")
for t_ in "KV":
    wait_loaded(t_, X0, Y - 1, Z0, secs=90)
both_(f"gamemode creative KilnTest")
E(f"tp KilnTest {X0 + 20} {Y + 2} {Z0 + 20}")
time.sleep(2)
E(f"fill {X0 - 16} {Y - 1} {Z0 - 16} {X0 + 16} {Y + 12} {Z0 + 16} minecraft:air")
E(f"fill {X0 - 16} {Y - 1} {Z0 - 16} {X0 + 16} {Y - 1} {Z0 + 16} minecraft:stone")
E(f"fill {X0 - 17} {Y} {Z0 - 17} {X0 + 17} {Y + 8} {Z0 + 17} minecraft:barrier hollow")
time.sleep(1)
FEATS = (("rider", "nbt={Passengers:[{}]}"), ("baby", "nbt={IsBaby:1b}"), ("main", "nbt={equipment:{mainhand:{}}}"), ("off", "nbt={equipment:{offhand:{}}}"),
         ("head", "nbt={equipment:{head:{}}}"), ("chest", "nbt={equipment:{chest:{}}}"), ("legs", "nbt={equipment:{legs:{}}}"), ("feet", "nbt={equipment:{feet:{}}}"))
print(f"{'type':18s}{'N':>5}  K | V    (a feature: K count / V count)", flush=True)
for ty in TYPES:
    for t in "KV":
        w(t, f"execute in minecraft:overworld run kill @e[type=!player,{BOX}]")
    time.sleep(0.8)
    for i in range(N):
        E(f"summon minecraft:{ty} {X0 + (i % 25) - 12}.5 {Y} {Z0 + (i // 25 % 25) - 12}.5")
        if i % 60 == 59:
            time.sleep(0.4)
    time.sleep(2.0)
    row = {t: {} for t in "KV"}
    for t in "KV":
        row[t]["total"] = tagn(t, f"@e[type=minecraft:{ty},{BOX}]", "jc")
        for name, test in FEATS:
            row[t][name] = tagn(t, f"@e[type=minecraft:{ty},{test},{BOX}]", "f_" + name)
        # the riders: everything near that is not a tagged mount - by type
        others = {}
        for rty in ("zombie", "skeleton", "parched", "drowned", "husk", "chicken", "spider", "cave_spider", "stray", "bogged", "zombie_villager", "piglin", "pillager", "vindicator", "illusioner", "vex", "villager"):
            if rty == ty:
                continue
            c = kills(t, f"@e[type=minecraft:{rty},{BOX}]")
            if c:
                others[rty] = c
        row[t]["riders"] = others
        # the mount types that carried them: kill what is left (tagged j and untagged)
        kills(t, f"@e[type=!player,{BOX}]")
    f = [n for n, _ in FEATS if row["K"][n] or row["V"][n]]
    line = f"{ty:18s}{N:>5}  tot {row['K']['total']}|{row['V']['total']}  " + " ".join(f"{n} {row['K'][n]}|{row['V'][n]}" for n in f)
    line += f"  riders K {row['K']['riders']} V {row['V']['riders']}"
    print(line, flush=True)
both_("execute in minecraft:overworld run kill @e[type=!player]")
