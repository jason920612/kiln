"""wp80: a gallery of hostile mobs against a survival player in a closed arena (barrier walls), both servers at once with real clients: which packet kinds / sounds /
particles / effects the client gets. Prints, per mob, what only Kiln sent and what only vanilla sent (after SECS seconds). Arg 1: comma list of mobs, arg 2: seconds, arg 3: extra NBT."""
from lab80 import *
import collections
MOBS = (sys.argv[1] if len(sys.argv) > 1 else "zombie,husk,skeleton,stray,bogged,wither_skeleton,spider,cave_spider,creeper,witch,pillager,vindicator,evoker,vex,ravager,ghast,blaze,magma_cube,slime,piglin,piglin_brute,hoglin,zoglin,zombified_piglin,breeze,silverfish,endermite,warden,iron_golem,wolf,polar_bear,llama,bee,fox,goat,panda,dolphin,axolotl,drowned,phantom,enderman,shulker,wither,creaking").split(",")
SECS = float(sys.argv[2]) if len(sys.argv) > 2 else 12
EXTRA = sys.argv[3] if len(sys.argv) > 3 else ""
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false", "mob_griefing false", "do_daylight_cycle false"))
both_("difficulty normal")
both_("time set midnight")
X, Y, Z = 30, 230, 30
revive()
cl = clients()
for t in "KV":
    w(t, "gamemode survival KilnTest")
OB(f"fill {X - 11} {Y - 1} {Z - 11} {X + 11} {Y + 8} {Z + 11} minecraft:barrier hollow")
OB(f"fill {X - 10} {Y - 1} {Z - 10} {X + 10} {Y - 1} {Z + 10} minecraft:stone")
SKIP = {"normalize"}
def sig(l):
    c = l.split(" ")[0]
    if c in ("entity_data", "Pos", "PosRot", "Rot", "PlayerPosition", "SystemChat", "attributes", "SetTime", "update_time", "Time"):
        return None
    if c == "sound":
        return "sound " + l.split(" ")[1]
    return re.sub(r"-?\d+(\.\d+)?(E-?\d+)?", "#", l)[:70]
for mob in MOBS:
    revive()
    both_("gamemode survival KilnTest")
    both_("kill @e[type=!player]")
    both_("effect clear KilnTest")
    both_("attribute KilnTest minecraft:max_health base set 1000")
    both_("effect give KilnTest minecraft:instant_health 1 10 true")
    OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
    time.sleep(1.0)
    for c in cl.values():
        c.cmd("spy on"); c.cmd("spylog clear")
    OB(f"summon minecraft:{mob} {X + 6}.5 {Y} {Z}.5 {{PersistenceRequired:1b{EXTRA}}}")
    time.sleep(SECS)
    S = {}
    for k, c in cl.items():
        lines = json.loads(c.cmd("spylog clear"))
        cnt = collections.Counter(s for s in map(sig, lines) if s)
        S[k] = cnt
    onlyK = sorted(set(S["K"]) - set(S["V"]))
    onlyV = sorted(set(S["V"]) - set(S["K"]))
    print(f"{mob:18s} K:{sum(S['K'].values()):4d} V:{sum(S['V'].values()):4d}  onlyK {onlyK[:8]}  onlyV {onlyV[:8]}", flush=True)
both_("kill @e[type=!player]")
