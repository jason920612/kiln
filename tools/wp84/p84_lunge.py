"""wp84: one lunge jab, packets and client positions every 50 ms. par.sh p84_lunge.py [level] [pitch]"""
from p72 import *
import collections
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
LV = sys.argv[1] if len(sys.argv) > 1 else "1"
PITCH = sys.argv[2] if len(sys.argv) > 2 else "0"
scenario("lunge84")
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("difficulty normal", show=False)
platform(0, 0, 40, 60, y=199, clear=14)
kill_all(); kit(ME)
sv(f"effect give {ME} minecraft:saturation 99999 0 true", show=False)
sv(f"give {ME} minecraft:diamond_spear[enchantments={{\"minecraft:lunge\":{LV}}}] 1", show=False)
time.sleep(0.5)
a.select("minecraft:diamond_spear")
sv(f"tp {ME} 20.5 200 10.5", show=False)
time.sleep(2.0)
a.look(0, float(PITCH))
time.sleep(0.5)
a.cmd("spy on"); a.cmd("spylog clear")
z0 = pos(a)[2]
t0 = time.time()
a.mouse("left", "tap")
trk = []
while time.time() - t0 < 1.6:
    s = a.state(0)
    trk.append((round(time.time() - t0, 2), round(s["z"] - z0, 3)))
    time.sleep(0.05)
ls = json.loads(a.cmd("spylog clear"))
F("lunge", LV, "pitch", PITCH, "z track (s, dz):", trk[:14])
F("final dz", round(pos(a)[2] - z0, 3))
F("kinds", collections.Counter(l.split(" ")[0] for l in ls))
for l in ls:
    if l.startswith(("motion", "PlayerPos", "Teleport", "Explode", "explode")):
        F("pkt", re.sub(r"\s+", " ", l)[:200])
