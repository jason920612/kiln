"""wp78: block counts of a chunk of the overworld by `fill ... replace`, on Kiln, the vanilla reference and a second, fresh vanilla world (determinism of vanilla itself)."""
import re, sys, time
from pathlib import Path
H = Path.home() / "wp78"
S = {"K": (H / "console", H / "server.log"), "V1": (H / "vanilla" / "console", H / "vanilla" / "server.log"), "V2": (H / "v2" / "console", H / "v2" / "server.log")}
ANSI = re.compile(r"\x1b\[[0-9;]*m")
def ask(t, cmd, wait=0.6):
    con, log = S[t]
    n = log.stat().st_size
    with open(con, "w") as f: f.write(cmd + "\n")
    time.sleep(wait)
    with open(log, "rb") as f:
        f.seek(n); return ANSI.sub("", f.read().decode(errors="replace"))
def num(o):
    m = re.search(r"filled\[(\d+)\]|filled (\d+) block|(\d+) block", o)
    return int(next(g for g in m.groups() if g)) if m else 0
cx, cz = int(sys.argv[1]), int(sys.argv[2])
x0, z0 = cx * 16, cz * 16
for t in S:
    ask(t, f"execute in minecraft:overworld run forceload add {x0} {z0} {x0 + 15} {z0 + 15}")
for t in S:
    for i in range(60):
        o = ask(t, f"execute in minecraft:overworld run setblock {x0} -64 {z0} minecraft:bedrock", wait=1.0)
        if "not loaded" not in o and "unloaded" not in o and "outside" not in o: break
res = {t: {} for t in S}
for blk in ("sculk", "sculk_vein", "sculk_sensor", "sculk_catalyst", "sculk_shrieker", "deepslate_bricks", "chest"):
    row = []
    for t in S:
        o = ask(t, f"execute in minecraft:overworld run fill {x0} -63 {z0} {x0 + 15} -10 {z0 + 15} minecraft:glass replace minecraft:{blk}", wait=1.0)
        res[t][blk] = num(o)
        row.append(num(o))
    print(f"{blk:18s} K {row[0]:5d}  V1 {row[1]:5d}  V2 {row[2]:5d}", flush=True)
