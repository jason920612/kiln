from lock import *
import re
X0, Y0, Z0, W, H, D = 10, 200, 10, 9, 4, 9
CMDS = """setblock 15 200 14 minecraft:slime_block
setblock 15 201 14 minecraft:slime_block
setblock 16 201 14 minecraft:sticky_piston[facing=west]
setblock 16 201 13 minecraft:stone
setblock 17 201 13 minecraft:tnt
setblock 14 203 15 minecraft:oak_trapdoor[facing=east,half=bottom,open=false]
setblock 16 203 13 minecraft:powered_rail[shape=east_west]""".split("\n")
import sys
if len(sys.argv) > 1:
    CMDS = [CMDS[int(i)] for i in sys.argv[1].split(",")]
both_("tick freeze")
both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
both_("kill @e[type=!player]")
both_("tick step 30"); time.sleep(2.2)
for c in CMDS: both_(c)
time.sleep(0.5)
for step in (1, 3, 8, 20, 60):
    both_(f"tick step {step}"); time.sleep(0.4 + 0.06 * step)
    for t in "KV":
        w(t, "execute as @e[type=!player] run data get entity @s Pos")
    time.sleep(1.0)
sel = "@e[type=!player]"
for t in "KV":
    n = logpos(t)
    w(t, f"execute as {sel} run data get entity @s Item")
    time.sleep(0.8)
    txt = since(t, n)
    ids = re.findall(r"id: \"minecraft:([a-z_]+)\"", txt)
    n = logpos(t)
    w(t, f"execute as {sel} run data get entity @s id")
    time.sleep(0.8)
    types = re.findall(r"minecraft:([a-z_]+)", since(t, n))
    print(t, "item ids", ids, "entities", types)
both_("tick unfreeze")
