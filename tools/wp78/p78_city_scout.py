"""wp78 item 4a: scout the ancient city at (816,~,-576) on both servers (par.sh): creative client, census of sculk blocks, chests, reinforced deepslate
inside a box all chunks of which both clients have loaded; a digest of the sorted positions per block name."""
import hashlib
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv(f"gamemode creative {ME}", show=False)
sv(f"effect give {ME} minecraft:night_vision 99999 0 true", show=False)
sv(f"effect give {ME} minecraft:slow_falling 99999 0 true", show=False)
sv(f"execute in minecraft:overworld run tp {ME} 816 -30 -576", show=False)
time.sleep(45)
st = a.state(0)
print("at", round(st["x"]), round(st["y"]), round(st["z"]))
R = 40
tot = {}
for nm in ("sculk_shrieker", "sculk_sensor", "sculk_catalyst", "chest", "reinforced_deepslate", "soul_lantern", "candle", "sculk_vein", "sculk", "deepslate_tile"):
    r = sorted(map(tuple, a.findbox(nm, 816 - R, -62, -576 - R, 816 + R, 10, -576 + R, 3000)))
    h = hashlib.md5(str(r).encode()).hexdigest()[:8]
    print(f"{nm:22s} {len(r):5d}  digest {h}")
    tot[nm] = r
sh = tot["sculk_shrieker"]
print("shriekers:", [(b, a.block(*b)[26:70]) for b in sh[:12]])
open(f"/tmp/city_{T}.txt", "w").write(repr(tot))
