"""wp78: scout a structure with a creative client on both servers (par.sh): tp to X Y Z in DIM, wait for the chunks, list blocks matching SUBSTR near the player.
usage: p78_scout.py DIM X Y Z SUBSTR [R]"""
import sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
dim, x, y, z, sub = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
r = int(sys.argv[6]) if len(sys.argv) > 6 else 40
sv("gamemode creative KilnTest", show=False)
sv(f"effect give KilnTest minecraft:slow_falling 600 0 true", show=False)
sv(f"effect give KilnTest minecraft:fire_resistance 600 0 true", show=False)
sv(f"execute in minecraft:{dim} run tp KilnTest {x} {y} {z}", show=False)
time.sleep(10)
st = a.state(0)
print("at", round(st["x"],1), round(st["y"],1), round(st["z"],1), "dim?", st.get("dim"))
res = a.find(sub, r, 400)
print(len(res), "blocks", sub)
ys = {}
for b in res:
    ys.setdefault(b[1] if isinstance(b, list) else b.get("y"), 0)
    ys[b[1] if isinstance(b, list) else b.get("y")] += 1
print("y histogram", sorted(ys.items()))
print(res[:12])
