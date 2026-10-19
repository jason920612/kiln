"""wp84: an ominous item spawner summoned by command: what the client is told (entity data with the item), and what appears. par.sh p84_ois.py"""
from p72 import *
import collections
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
scenario("ois84")
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("difficulty normal", show=False)
platform(0, 0, 40, 60, y=199, clear=14)
kill_all(); kit(ME)
sv(f"tp {ME} 20.5 200 20.5", show=False)
time.sleep(2.0)
a.cmd("spy on"); a.cmd("spylog clear")
for item in ("minecraft:arrow", "minecraft:wind_charge", "minecraft:splash_potion"):
    sv(f'summon minecraft:ominous_item_spawner 20.5 204 25.5 {{item:{{id:"{item}",count:1}},spawn_item_after_ticks:40}}', show=False)
    time.sleep(0.5)
time.sleep(4.0)
ls = json.loads(a.cmd("spylog clear"))
for l in ls:
    if not l.startswith(("Pos", "Rot", "PlayerPos", "Time", "SetTime", "update_time", "player_info")):
        F(re.sub(r"-?\d+\.\d+", "#", l)[:160])
F("entities:", collections.Counter(e["t"] for e in a.state(40)["entities"]))
kill_all()
