"""wp80: the survival player falls from H blocks onto stone: the particle / sound / health packets his client gets, both servers (real clients). usage: p80_fall.py H [lines pattern]"""
from lab80 import *
import collections
H = int(sys.argv[1]) if len(sys.argv) > 1 else 10
pat = sys.argv[2] if len(sys.argv) > 2 else "particle|sound|SetHealth|DamageEvent"
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 30, 230, 30
revive()
cl = clients()
OB(f"fill {X - 11} {Y - 1} {Z - 11} {X + 11} {Y + 40} {Z + 11} minecraft:air")
OB(f"fill {X - 10} {Y - 1} {Z - 10} {X + 10} {Y - 1} {Z + 10} minecraft:stone")
both_("gamemode survival KilnTest"); both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:instant_health 1 10 true")
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
time.sleep(2)
for c in cl.values():
    c.cmd("spy on"); c.cmd("spylog clear")
OB(f"tp KilnTest {X}.5 {Y + H} {Z}.5")
time.sleep(4)
for k, c in cl.items():
    lines = json.loads(c.cmd("spylog clear"))
    cnt = collections.Counter(re.sub(r"-?\d+\.\d+(E-?\d+)?", "#", l)[:120] for l in lines if re.search(pat, l))
    print(k, c.state(0)["health"])
    for l, n in cnt.most_common(10):
        print("   ", n, l)
