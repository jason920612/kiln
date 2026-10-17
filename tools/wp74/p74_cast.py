"""wp74: one cast into the closed pond, bobber track every 0.1 s (par.sh)."""
import sys
from p72 import *
import p61
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
for c in ("tick rate 20",):
    sv(c, show=False)
sv(f"tp {ME} 16.5 200 21.5", show=False)
time.sleep(2)
a.lookat(16.5, 199.5, 16.5)
for k in range(3):
    a.mouse("right", "tap")
    tr = []
    t0 = time.time()
    while time.time() - t0 < 4:
        b = [e for e in a.state(30)["entities"] if e["t"] == "fishing_bobber"]
        tr.append((round(time.time() - t0, 1), (round(b[0]["x"], 2), round(b[0]["y"], 2), round(b[0]["z"], 2)) if b else None))
        time.sleep(0.4)
    print(T, "cast", k, tr, flush=True)
    a.mouse("right", "tap"); time.sleep(1.0)
print(T, "water at 16,199,16:", p61.blk(a, 16, 199, 16), p61.blk(a, 16, 198, 16), "walls", p61.blk(a, 14, 199, 16), p61.blk(a, 18, 199, 16), "player", p61.me(a))
