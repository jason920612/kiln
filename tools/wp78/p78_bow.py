"""wp78: real bow shots at a stationary NoAI wither (par.sh): damage per shot, K against V. DIST=blocks (default 12)."""
import sys
from p72 import *
import p61
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
CX, CY, CZ = 500, 200, 500
DIST = float(os.environ.get("DIST", "12"))
TYPE = sys.argv[1] if len(sys.argv) > 1 else "wither"

def hp():
    o = nbt_of(f"@e[type={TYPE},limit=1]", "Health", wait=0.35)
    m = re.findall(r"(\d+(?:\.\d+)?)f", o)
    return float(m[-1]) if m else None

if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
p61.platform(CX - 24, CZ - 24, CX + 24, CZ + 24, y=199, clear=12)
kill_all()
sv("gamerule mob_griefing false", show=False)
sv(f"gamemode survival {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"give {ME} minecraft:bow[enchantments={{\"minecraft:power\":5}}]", show=False)
sv(f"give {ME} minecraft:arrow 64", show=False)
sv(f"effect give {ME} minecraft:resistance 600 4 true", show=False)
sv(f"summon minecraft:{TYPE} {CX}.5 {CY} {CZ}.5 {{NoAI:1b,Invul:0,PersistenceRequired:1b,Silent:1b}}", show=False)
sv(f"tp {ME} {CX}.5 {CY} {CZ + DIST}.5", show=False)
time.sleep(2.5)
a.select("minecraft:bow")
res = []
for i in range(12):
    w0 = [e for e in a.state(60)["entities"] if e["t"] == TYPE]
    if not w0:
        print("no wither"); break
    h0 = hp()
    a.shoot_ballistic(w0[0]["x"], w0[0]["y"] + 1.5, w0[0]["z"], charge=1.3)
    time.sleep(1.4)
    h1 = hp()
    res.append(round(h0 - h1, 2))
print("damage per shot:", res, "hits:", sum(1 for r in res if r > 0), "of", len(res))
shot("bow_" + T)
