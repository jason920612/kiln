"""wp78: is the explosion destroying random between repeats? 6 TNT explosions at the same site in a dirt/glass/plank wall, counts of survivors per trial (lock step)."""
from lock import *
import collections
CX, CY, CZ = 500, 200, 500
CL = {"K": 25999, "V": 26000}
def wall(t):
    c = C(CL[t]); d = collections.Counter()
    for l in json.loads(c.cmd("boxdump %d %d %d %d %d %d" % (CX - 12, CY, CZ - 12, CX + 12, CY + 6, CZ + 12))):
        d[l.split(" ", 3)[3].split("[")[0].replace("minecraft:", "")] += 1
    return sum(v for k, v in d.items() if k not in ("bedrock", "tnt"))
respawn_all()
setup(frozen=False, rules=("mob_griefing true", "command_block_output false", "tnt_explodes true"))
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, f"forceload add {CX-30} {CZ-30} {CX+30} {CZ+30}"); w(t, f"tp KilnTest {CX+20} {CY+6} {CZ+20}")
time.sleep(8)
for t in "KV": w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
res = {"K": [], "V": []}
for trial in range(6):
    both_("tick unfreeze")
    for t in "KV":
        w(t, "kill @e[type=!player]")
        w(t, f"fill {CX-14} {CY-1} {CZ-14} {CX+14} {CY+10} {CZ+14} minecraft:air")
        w(t, f"fill {CX-14} {CY-1} {CZ-14} {CX+14} {CY-1} {CZ+14} minecraft:bedrock")
        w(t, f"fill {CX-6} {CY} {CZ-6} {CX+6} {CY+5} {CZ+6} minecraft:dirt hollow")
        w(t, f"fill {CX-2} {CY} {CZ-2} {CX+2} {CY+2} {CZ+2} minecraft:air")
    time.sleep(2)
    both_("tick freeze"); time.sleep(0.5)
    both_(f"summon minecraft:tnt {CX}.5 {CY+0.0} {CZ}.5 {{fuse:20,Tags:[\"x\"]}}")
    step(25)
    time.sleep(0.5)
    n = {t: wall(t) for t in "KV"}
    for t in "KV": res[t].append(n[t])
    print("trial", trial, n, flush=True)
both_("tick unfreeze")
print(res)
