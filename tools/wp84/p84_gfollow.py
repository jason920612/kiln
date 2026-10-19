"""wp84: ghastling brain under the microscope: the server's Pos and Brain memories every 1 s while the player stands still holding snowballs / a stick / nothing.
usage: par.sh p84_gfollow.py [dist]"""
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
scenario("gfollow84")
DIST = float(sys.argv[1]) if len(sys.argv) > 1 else 9.0
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "do_daylight_cycle false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=14)
for label, item in (("snowball", "minecraft:snowball"), ("stick", "minecraft:stick"), ("hand", None)):
    kill_all()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    if item:
        give(ME, item, 16)
        a.select(item)
    else:
        a.hotbar(9)
    sv(f"summon minecraft:happy_ghast 20.5 202 {20.5 + DIST} {{Age:-24000,PersistenceRequired:1b}}", show=False)
    time.sleep(0.5)
    rows = []
    for i in range(10):
        time.sleep(1.0)
        pos_ = re.findall(r"-?\d+\.\d+", nbt_of("@e[type=happy_ghast,limit=1]", "Pos", 0.35))[:3]
        mem = nbt_of("@e[type=happy_ghast,limit=1]", "Brain.memories", 0.35)
        keys = sorted(set(re.findall(r'"?(minecraft:[a-z_]+)"?\s*:', mem)))
        rows.append((i + 1, [round(float(p), 1) for p in pos_], [k.replace("minecraft:", "") for k in keys]))
    for r in rows:
        F(label, r)
kill_all()
