"""wp78: do item entities die in an explosion? (console only, both servers in lock step) Items of several kinds at several distances around a TNT: survivors after the blast."""
from lock import *
import re
X, Y, Z = 700, 200, 700
setup(frozen=False, rules=("mob_griefing false", "command_block_output false"))
both_(f"execute in minecraft:overworld run forceload add {X - 20} {Z - 20} {X + 20} {Z + 20}")
for t in "KV":
    wait_loaded(t, X + 15, Y + 8, Z + 15, secs=120)
both_("tick freeze")
KINDS = ["stone", "dirt", "oak_planks", "diamond", "obsidian", "netherite_ingot", "tnt", "iron_ingot"]
for trial in range(3):
    both_("kill @e[type=!player]")
    both_(f"execute in minecraft:overworld run fill {X - 12} {Y - 1} {Z - 12} {X + 12} {Y + 6} {Z + 12} minecraft:air")
    both_(f"execute in minecraft:overworld run fill {X - 12} {Y - 1} {Z - 12} {X + 12} {Y - 1} {Z + 12} minecraft:bedrock")
    time.sleep(1.0)
    for i, k in enumerate(KINDS):
        for d in (1, 2, 3, 5, 7):
            both_(f"execute in minecraft:overworld run summon minecraft:item {X + 0.5 + (d if i % 2 == 0 else -d)} {Y + 0.1} {Z + 0.5 + (i - 4) * 1.0} {{Item:{{id:'minecraft:{k}',count:1}},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767s,Tags:['i_{k}_{d}']}}")
    both_(f"execute in minecraft:overworld run summon minecraft:tnt {X + 0.5} {Y + 0.5} {Z + 0.5} {{fuse:2}}")
    step(5)
    res = {}
    for t in "KV":
        r = {}
        for k in KINDS:
            for d in (1, 2, 3, 5, 7):
                r[(k, d)] = count(t, f"@e[tag=i_{k}_{d}]")
        res[t] = r
    for k in KINDS:
        print(trial, f"{k:16s}", "K", "".join(str(res['K'][(k, d)]) for d in (1, 2, 3, 5, 7)), "V", "".join(str(res['V'][(k, d)]) for d in (1, 2, 3, 5, 7)), flush=True)
both_("tick unfreeze")
