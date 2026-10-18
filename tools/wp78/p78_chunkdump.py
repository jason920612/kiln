"""wp78: dump a chunk column's blocks as the client sees them (par.sh) to /tmp/chunk_<T>.json; args: CX CZ Y0 Y1"""
import sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
cx, cz, y0, y1 = (int(v) for v in sys.argv[1:5])
x0, z0 = cx * 16, cz * 16
if T == "V":
    sv("op KilnTest", show=False)
sv("gamemode creative KilnTest", show=False)
sv(f"execute in minecraft:overworld run tp KilnTest {x0 + 8} {y0 + 10} {z0 + 8}", show=False)
time.sleep(25)
rows = json.loads(a.cmd("boxdump %d %d %d %d %d %d" % (x0, y0, z0, x0 + 15, y1, z0 + 15)))
open(f"/tmp/chunk_{T}.json", "w").write(json.dumps(rows))
print(T, "blocks", len(rows))
