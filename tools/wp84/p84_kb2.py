"""wp84: knockback motion packets with the tick boundaries controlled (tick freeze + step), both servers, both real clients (lock-step lab)."""
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
OB(f"summon minecraft:zombie {X + 3}.5 {Y} {Z}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"zz\"]}}")
time.sleep(1.0)
def run(label, cmd_fn, steps=3):
    both_("tick freeze")
    time.sleep(0.5)
    both_(f"execute in minecraft:overworld run tp KilnTest {X}.5 {Y} {Z}.5")
    step(3)
    for c in cl.values():
        c.cmd("spylog clear")
    cmd_fn()
    out = {}
    for i in range(steps):
        step(1)
        time.sleep(0.3)
        for t, c in cl.items():
            ls = json.loads(c.cmd("spylog clear"))
            out.setdefault(t, []).extend([f"t{i+1} " + l[12:] for l in ls if l.startswith("motion_self")])
    both_("tick unfreeze")
    print(label)
    for t in "KV":
        print("  ", t, out.get(t))
for c in cl.values():
    c.cmd("spy on")
run("damage by zombie (command, frozen, on ground)", lambda: OB("damage KilnTest 1 minecraft:mob_attack by @e[tag=zz,limit=1]"))
run("damage by zombie again", lambda: OB("damage KilnTest 1 minecraft:mob_attack by @e[tag=zz,limit=1]"))
run("damage generic (no knockback source)", lambda: OB("damage KilnTest 1 minecraft:generic"))
both_("kill @e[type=!player]")
