from lab80 import *
import collections
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("kill @e[type=!player]")
cl = clients()
X,Y,Z=30,200,30
for t in "KV": w(t, "gamemode survival KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest instant_health 1 30 true")
for c in cl.values():
    c.cmd("spy on"); c.cmd("spylog clear")
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
OB(f"summon minecraft:skeleton {X+6}.5 {Y} {Z}.5 {{PersistenceRequired:1b}}")
time.sleep(12)
for k,c in cl.items():
    lines = json.loads(c.cmd("spylog clear"))
    print(k, len(lines))
    seen=set()
    for l in lines:
        key=re.sub(r"-?\d+\.\d+","#",l)[:110]
        if key in seen: continue
        seen.add(key); print("   ", key)
both_("kill @e[type=!player]")
