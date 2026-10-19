"""p84_one.py mob secs [nbt]: one mob against the survival player in the barrier arena; health of both players every second (log of damage)"""
from lab84 import *
mob = sys.argv[1]; secs = int(sys.argv[2]); extra = sys.argv[3] if len(sys.argv) > 3 else ""
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false", "mob_griefing false", "do_daylight_cycle false"))
both_("difficulty normal"); both_("time set midnight")
X, Y, Z = 30, 230, 30
revive()
OB(f"fill {X - 11} {Y - 1} {Z - 11} {X + 11} {Y + 8} {Z + 11} minecraft:barrier hollow")
OB(f"fill {X - 10} {Y - 1} {Z - 10} {X + 10} {Y - 1} {Z + 10} minecraft:stone")
both_("gamemode survival KilnTest"); both_("kill @e[type=!player]"); both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:instant_health 1 10 true")
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
time.sleep(1.5)
print("start", {t: health(t) for t in "KV"})
OB(f"summon minecraft:{mob} {X + 6}.5 {Y} {Z}.5 {{PersistenceRequired:1b{extra}}}")
for s in range(secs):
    time.sleep(1)
    print(s + 1, {t: health(t) for t in "KV"}, flush=True)
both_("kill @e[type=!player]")
