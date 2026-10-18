from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("kill @e[type=!player]")
both_("tick unfreeze")
Y = 100
bx, bz = 40, 40
OB(f"fill {bx-6} {Y - 2} {bz-6} {bx+8} {Y + 8} {bz+6} minecraft:air")
OB(f"fill {bx-6} {Y - 1} {bz-6} {bx+8} {Y - 1} {bz+6} minecraft:stone")
for x in (bx, bx + 3):
    OB(f"fill {x} {Y} {bz} {x} {Y + 4} {bz} minecraft:obsidian")
OB(f"fill {bx} {Y} {bz} {bx+3} {Y} {bz} minecraft:obsidian")
OB(f"fill {bx} {Y + 4} {bz} {bx+3} {Y + 4} {bz} minecraft:obsidian")
OB(f"fill {bx+1} {Y + 1} {bz} {bx+2} {Y + 3} {bz} minecraft:nether_portal[axis=x]")
time.sleep(1)
OB(f"summon minecraft:item {bx+1}.5 {Y + 1} {bz}.5 {{Item:{{id:\"minecraft:stone\",count:2}},Tags:[\"np_item\"],PickupDelay:20s}}")
OB(f"summon minecraft:pig {bx+2}.5 {Y + 1} {bz}.5 {{NoAI:1b,Tags:[\"np_pig\"],PersistenceRequired:1b}}")
def where(t, tag):
    r = []
    for dim in ("overworld", "the_nether"):
        o = ask(t, f"execute in minecraft:{dim} positioned 0 0 0 run data get entity @e[tag={tag},distance=..10000000,limit=1] Pos", wait=0.3)
        m = re.findall(r"(-?\d+(?:\.\d+)?)d", o)
        if len(m) >= 3:
            r.append((dim[:4], [round(float(v), 1) for v in m[:3]]))
    return r
for i in range(10):
    time.sleep(1.0)
    for t in "KV":
        print(i, t, "item", where(t, "np_item"), "pig", where(t, "np_pig"), flush=True)
both_("kill @e[type=!player]")
