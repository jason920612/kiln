"""wp76: Kiln's 600-tick reports (`KILN_PHASE_DETAIL=1`) from a server.log, with the scene that ran then.

python3 phases76.py SERVER_LOG [TOP]   prints, per report: time, mspt mean, the scene marks seen before it, the
                                       biggest parts (ms per tick)
"""
import re
import sys

log = sys.argv[1]
top = int(sys.argv[2]) if len(sys.argv) > 2 else 30
scene = "-"
for line in open(log, encoding="utf-8", errors="replace"):
    m = re.search(r"Running function wp76:(\w+)", line)
    if m:
        scene = m.group(1)
    if "mspt mean" not in line:
        continue
    t = line[11:19]
    rest = line.split("max ", 1)[1].partition("| ")[2]
    mean = re.search(r"mspt mean ([\d.]+)", line).group(1)
    parts = []
    for p in rest.split(" | "):
        p = p.strip().rsplit(" ", 1)
        if len(p) == 2:
            try:
                parts.append((float(p[1]), p[0]))
            except ValueError:
                pass
    parts.sort(reverse=True)
    print(f"{t} scene {scene:12s} mean {mean}")
    print("   " + "  ".join(f"{n} {v:.3f}" for v, n in parts[:top]))
