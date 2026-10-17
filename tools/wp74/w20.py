from lock import *
import re
X0, Y0, Z0 = 100, 200, 100
for t in "KV":
    w(t, "kill @e[type=turtle]")
time.sleep(1)
for t in "KV":
    w(t, f"summon minecraft:turtle {X0 + 52}.5 {Y0 - 2} {Z0 + 52}.5 {{PersistenceRequired:1b,Tags:[\"mon\"]}}")
def nums(o):
    return re.findall(r"-?\d+\.\d+", o)
for step in range(30):
    for t in "KV":
        sprint(t, 40)
    row = {}
    for t in "KV":
        pos = ask(t, 'data get entity @e[tag=mon,limit=1] Pos', wait=0.2)
        hp = ask(t, 'data get entity @e[tag=mon,limit=1] Health', wait=0.2)
        fd = ask(t, 'data get entity @e[tag=mon,limit=1] fall_distance', wait=0.2)
        row[t] = ([round(float(x)) for x in nums(pos)[-3:]], nums(hp)[-1:] , nums(fd)[-1:], count(t, "@e[tag=mon]"))
    print(step * 40 + 40, "K", row["K"], "V", row["V"], flush=True)
