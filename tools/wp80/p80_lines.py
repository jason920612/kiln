"""p80_lines.py mob secs pattern [nbt]: print the distinct spy lines matching pattern on both clients while a mob works on the player"""
from lab80 import *
import collections
mob = sys.argv[1]; secs = float(sys.argv[2]); pat = sys.argv[3]; extra = sys.argv[4] if len(sys.argv) > 4 else ""
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false", "mob_griefing false", "do_daylight_cycle false"))
both_("difficulty normal"); both_("time set midnight")
X, Y, Z = 30, 230, 30
revive()
cl = clients()
OB(f"fill {X - 11} {Y - 1} {Z - 11} {X + 11} {Y + 8} {Z + 11} minecraft:barrier hollow")
OB(f"fill {X - 10} {Y - 1} {Z - 10} {X + 10} {Y - 1} {Z + 10} minecraft:stone")
both_("gamemode survival KilnTest"); both_("kill @e[type=!player]"); both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:instant_health 1 10 true")
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
time.sleep(1.5)
for c in cl.values():
    c.cmd("spy on"); c.cmd("spylog clear")
OB(f"summon minecraft:{mob} {X + 6}.5 {Y} {Z}.5 {{PersistenceRequired:1b{extra}}}")
time.sleep(secs)
for k, c in cl.items():
    lines = json.loads(c.cmd("spylog clear"))
    cnt = collections.Counter(l[:200] for l in lines if re.search(pat, l))
    print(k, len(cnt), flush=True)
    for l, n in cnt.most_common(12):
        print("   ", n, l)
both_("kill @e[type=!player]")
