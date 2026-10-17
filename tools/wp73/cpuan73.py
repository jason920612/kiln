"""wp73: the tick's own CPU time per tick from thr73.py samples (cpu73.sh): mean over seconds FROM..TO of the run of the
`sim` thread (serial phases, one region) and the `kiln-tick` workers, as ms per 50 ms tick; and the run's MSPT line.
usage: cpuan73.py DIR [FROM TO]"""
import ast, re, sys
from pathlib import Path
d = Path(sys.argv[1]); lo = int(sys.argv[2]) if len(sys.argv) > 2 else 60; hi = int(sys.argv[3]) if len(sys.argv) > 3 else 200
for thr in sorted(d.glob("*.thr")):
    rows = []
    for line in thr.read_text().splitlines():
        m = re.match(r"\S+ load (\S+) (\{.*\})", line)
        if m:
            rows.append((float(m.group(1)), ast.literal_eval(m.group(2))))
    win = rows[lo:hi]
    if not win:
        continue
    mean = lambda k: sum(r[1].get(k, 0.0) for r in win) / len(win)
    load = sum(r[0] for r in win) / len(win)
    log = thr.with_suffix(".log").read_text(errors="replace") if thr.with_suffix(".log").exists() else ""
    mspt = re.search(r"^mspt\s+mean ([\d.]+)\s+p50 ([\d.]+)\s+p99 ([\d.]+)\s+max ([\d.]+)", log, re.M)
    other = re.search(r"other programs about ([\d.]+) cores", log)
    print(f"{thr.stem:22s} tick CPU/tick: sim {mean('sim') * 50:5.2f} ms + workers {mean('kiln-tick') * 50:5.2f} ms = {(mean('sim') + mean('kiln-tick')) * 50:5.2f} ms; "
          f"gen {mean('kiln-gen'):4.2f} cores; load {load:5.2f}; other {other.group(1) if other else '?'} cores; "
          f"mspt {mspt.group(1) + '/' + mspt.group(3) + '/' + mspt.group(4) if mspt else '?'}")
