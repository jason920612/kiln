"""wp78: for one fuzz seed of out_dupe*.txt, build the case on both servers, step 32 ticks, list the item entities of the box (id, count, position) on each. usage: seed file"""
import ast, re, sys, time
from lock import *
seed, fn = sys.argv[1], sys.argv[2]
txt = open(fn).read()
m = re.search(r"DIFF seed=%s .*?\n.*?\n.*?\n  cmds: (\[.*?\])\n" % seed, txt, re.S)
cmds = ast.literal_eval(m.group(1))
X0, Y0, Z0, W, H, D = 10, 200, 10, 9, 4, 9
both_("tick freeze")
both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
both_("kill @e[type=!player]")
both_("tick step 20"); time.sleep(2)
for c in cmds:
    both_(c)
time.sleep(0.5)
for n in (1, 3, 8, 20):
    both_(f"tick step {n}"); time.sleep(0.4 + 0.06 * n)
time.sleep(1)
for t in "KV":
    out = ask(t, f"execute as @e[type=item,x={X0 - 3},y={Y0 - 3},z={Z0 - 3},dx={W + 6},dy={H + 80},dz={D + 6}] run data get entity @s Item", wait=1.5)
    items = re.findall(r"id: \"minecraft:([a-z_]+)\"", out) 
    cnts = re.findall(r"count: (\d+)", out)
    print(t, "items", list(zip(items, cnts)), flush=True)
    out2 = ask(t, f"execute as @e[type=item,x={X0 - 3},y={Y0 - 3},z={Z0 - 3},dx={W + 6},dy={H + 80},dz={D + 6}] run data get entity @s Pos", wait=1.5)
    print(t, "pos", re.findall(r"\[(-?[0-9.]+)d, (-?[0-9.]+)d, (-?[0-9.]+)d\]", out2), flush=True)
