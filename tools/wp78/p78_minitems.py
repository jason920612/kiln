"""wp78: delta-debug a fuzz case (seed, file) on the item entities (ids) left in the box after 32 ticks, Kiln against vanilla."""
import ast, collections, re, sys, time
from lock import *
seed, fn = sys.argv[1], sys.argv[2]
txt = open(fn).read()
m = re.search(r"DIFF seed=%s .*?\n.*?\n.*?\n  cmds: (\[.*?\])\n" % seed, txt, re.S)
cmds0 = ast.literal_eval(m.group(1))
X0, Y0, Z0, W, H, D = 10, 200, 10, 9, 4, 9
both_("tick freeze")
def items(t):
    out = ask(t, f"execute as @e[type=item,x={X0 - 3},y={Y0 - 3},z={Z0 - 3},dx={W + 6},dy={H + 80},dz={D + 6}] run data get entity @s Item", wait=1.2)
    return sorted(re.findall(r"id: \"minecraft:([a-z_]+)\"", out))
def case(cmds):
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 20"); time.sleep(2.0)
    for c in cmds: both_(c)
    time.sleep(0.4)
    for n in (1, 3, 8, 20):
        both_(f"tick step {n}"); time.sleep(0.4 + 0.06 * n)
    time.sleep(0.8)
    return items("K"), items("V")
k, v = case(cmds0)
print("full case: K", k, "V", v, flush=True)
assert k != v, "no difference"
cmds = list(cmds0)
changed = True
while changed:
    changed = False
    for i in range(len(cmds) - 1, -1, -1):
        trial = cmds[:i] + cmds[i + 1:]
        k, v = case(trial)
        if k != v:
            cmds, changed = trial, True
print("MINIMAL", len(cmds))
for c in cmds: print("  ", c)
k, v = case(cmds)
print("K", k, "V", v)
