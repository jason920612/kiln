"""wp80 item 1: elder guardian mining fatigue on a survival player: when the effect arrives, its parameters, the re-apply interval, the curse event packet.
Both servers, both players (lock.py), tick sprint between samples."""
from lab80 import *
import collections
TOTAL = int(sys.argv[1]) if len(sys.argv) > 1 else 7000
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 30, 200, 30
revive()
cl = clients()
for t in "KV":
    w(t, "gamemode survival KilnTest")
both_("kill @e[type=!player]")
both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
both_("effect give KilnTest minecraft:instant_health 1 20 true")
tank(X, Y, Z, 7, 4)
OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
time.sleep(2)
for c in cl.values():
    c.cmd("spy on"); c.cmd("spylog clear")
both_("tick freeze")
OB(f"summon minecraft:elder_guardian {X + 5}.5 {Y + 1} {Z + 5}.5 {{PersistenceRequired:1b,NoAI:0b}}")
def eff(t):
    o = ask(t, "data get entity KilnTest active_effects", wait=0.3)
    m = re.search(r"\{[^{}]*mining_fatigue[^{}]*\}", o)
    return m.group(0)[:160] if m else None
log = {t: [] for t in "KV"}
for tick in range(0, TOTAL, 100):
    for t in "KV":
        w(t, "tick step 100")
    time.sleep(0.5 + 0.055 * 100)
    for t in "KV":
        e = eff(t)
        d = re.search(r"duration\D*(\d+)", e) if e else None
        a = re.search(r"amplifier\D*(\d+)", e) if e else None
        log[t].append((tick + 100, int(d.group(1)) if d else None, int(a.group(1)) if a else None))
for t in "KV":
    print(t, "mining_fatigue (tick, duration, amplifier):", log[t], flush=True)
for k, c in cl.items():
    lines = json.loads(c.cmd("spylog clear"))
    ev = [l for l in lines if "GameEvent" in l or "game_event" in l.lower()]
    print(k, "game event lines:", len(ev), ev[:6], flush=True)
    print(k, "class counts:", dict(collections.Counter(l.split(" ")[0] for l in lines)), flush=True)
both_("tick unfreeze")
both_("kill @e[type=!player]")
