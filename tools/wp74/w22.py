from lock import *
import re
X0, Y0, Z0 = 100, 200, 100
for t in "KV":
    w(t, "tick unfreeze"); w(t, "kill @e[type=!player]")
    w(t, f"fill {X0} {Y0 - 1} {Z0} {X0 + 3} {Y0 + 4} {Z0 + 3} minecraft:air")
    w(t, f"fill {X0} {Y0 - 1} {Z0} {X0 + 3} {Y0 - 1} {Z0 + 3} minecraft:stone")
    w(t, f"setblock {X0 + 1} {Y0} {Z0 + 1} minecraft:cobweb")
time.sleep(1)
for t in "KV":
    w(t, f"summon minecraft:bat {X0 + 1}.5 {Y0} {Z0 + 1}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"b\"]}}")
    w(t, f"summon minecraft:bat {X0 + 3}.5 {Y0} {Z0 + 3}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"c\"]}}")
for step in range(8):
    for t in "KV":
        sprint(t, 25)
        w(t, "tick freeze")
    time.sleep(0.4)
    row = {}
    for t in "KV":
        row[t] = []
        for tag in "bc":
            hp = ask(t, f"data get entity @e[tag={tag},limit=1] Health", wait=0.15)
            row[t].append(re.findall(r"-?\d+\.\d+", hp)[-1:] or "gone")
        w(t, "tick unfreeze")
    print((step + 1) * 25, row)
