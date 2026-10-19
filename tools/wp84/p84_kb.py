"""wp84: the player's own motion packets (knockback) when a zombie hits a standing survival player. par.sh p84_kb.py"""
from p72 import *
import collections
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
scenario("kb84")
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "natural_regeneration false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 60, y=199, clear=14)
kill_all(); kit(ME)
sv(f"attribute {ME} minecraft:max_health base set 200", show=False)
sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
sv(f"tp {ME} 20.5 200 20.5", show=False)
time.sleep(2.0)
a.cmd("spy on"); a.cmd("spylog clear")
sv("summon minecraft:zombie 20.5 200 22.5 {PersistenceRequired:1b,Tags:[\"z\"]}", show=False)
time.sleep(6.0)
ls = json.loads(a.cmd("spylog clear"))
mot = [l for l in ls if l.startswith("motion_self")]
F("motion packets of the player in 6 s:", len(mot))
for l in mot[:8]:
    F("  ", l)
kill_all()
