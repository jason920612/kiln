"""wp84: a zombie hits the standing survival player; frozen server, one tick at a time; the player's own motion packets per tick, both servers."""
from lab84 import *
cl = clients()
setup(frozen=False, rules=("natural_regeneration false",))
both_("difficulty normal")
X, Y, Z = 30, 230, 30
revive()
OB(f"fill {X - 11} {Y - 1} {Z - 11} {X + 11} {Y + 8} {Z + 11} minecraft:barrier hollow")
OB(f"fill {X - 10} {Y - 1} {Z - 10} {X + 10} {Y - 1} {Z + 10} minecraft:stone")
for t in "KV":
    w(t, "gamemode survival KilnTest")
both_("kill @e[type=!player]")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:instant_health 1 10 true")
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
time.sleep(1.5)
for c in cl.values():
    c.cmd("spy on")
both_("tick freeze")
time.sleep(0.5)
OB(f"summon minecraft:zombie {X + 1}.2 {Y} {Z}.5 {{PersistenceRequired:1b,Tags:[\"zz\"]}}")
step(2)
for c in cl.values():
    c.cmd("spylog clear")
res = {"K": [], "V": []}
for i in range(60):
    step(1)
    for t, c in cl.items():
        ls = json.loads(c.cmd("spylog clear"))
        for l in ls:
            if l.startswith("motion_self") or l.startswith("HurtAnimation") or "hurt_animation" in l:
                res[t].append(f"t{i+1} " + l[:70])
both_("tick unfreeze")
for t in "KV":
    print(t, res[t][:12])
both_("kill @e[type=!player]")
