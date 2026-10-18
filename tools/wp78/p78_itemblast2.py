"""wp78: where does an explosion stop killing item entities? Items of one kind along +x from a TNT at 4.0..8.0 blocks (0.25 steps), K against V in lock step; survivors."""
from lock import *
X, Y, Z = 700, 200, 700
setup(frozen=False, rules=("mob_griefing false", "command_block_output false"))
both_(f"execute in minecraft:overworld run forceload add {X - 20} {Z - 20} {X + 20} {Z + 20}")
for t in "KV":
    wait_loaded(t, X + 15, Y + 8, Z + 15, secs=120)
both_("tick freeze")
DS = [4.0 + 0.25 * i for i in range(17)]
for trial in range(2):
    both_("kill @e[type=!player]")
    both_(f"execute in minecraft:overworld run fill {X - 12} {Y - 1} {Z - 12} {X + 12} {Y + 6} {Z + 12} minecraft:air")
    both_(f"execute in minecraft:overworld run fill {X - 12} {Y - 1} {Z - 12} {X + 12} {Y - 1} {Z + 12} minecraft:bedrock")
    time.sleep(1.0)
    for i, d in enumerate(DS):
        both_(f"execute in minecraft:overworld run summon minecraft:item {X + 0.5 + d} {Y} {Z + 0.5 + (i % 2) * 0.0 - 8 + i * 1.0} {{Item:{{id:'minecraft:stone',count:1}},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767s,Tags:['d{i}']}}")
    both_(f"execute in minecraft:overworld run summon minecraft:tnt {X + 0.5} {Y} {Z + 0.5} {{fuse:2}}")
    step(5)
    row = {t: "".join(str(count(t, f"@e[tag=d{i}]")) for i in range(len(DS))) for t in "KV"}
    print(trial, "distances", DS[0], "..", DS[-1], "\n  K", row["K"], "\n  V", row["V"], flush=True)
both_("tick unfreeze")
