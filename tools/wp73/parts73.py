"""Distribution of named parts in a server log's per-tick lines (KILN_SLOW_PRINT=0):
parts73.py LOG [PART...]: per part, how many ticks had it, mean, p50, p90, p99, max (ms)."""
import re, sys
log = sys.argv[1]
names = sys.argv[2:] or ["ch.unload_chunks", "ch.unload_entities", "ue.scan", "ue.save_in", "ue.leave", "ue.store", "uc.blocks", "uc.poi", "uc.save"]
vals = {n: [] for n in names}
ticks = 0
for line in open(log, errors="replace"):
    if "slow tick " not in line:
        continue
    ticks += 1
    for n in names:
        m = re.search(r"(?:: | \| )" + re.escape(n) + r" ([\d.]+)", line)
        if m:
            vals[n].append(float(m.group(1)))
print(f"{ticks} tick lines")
for n in names:
    v = sorted(vals[n])
    if not v:
        print(f"{n:20s} none")
        continue
    q = lambda p: v[min(len(v) - 1, int(p * len(v)))]
    print(f"{n:20s} n {len(v):5d}  mean {sum(v)/len(v):6.2f}  p50 {q(.5):6.2f}  p90 {q(.9):6.2f}  p99 {q(.99):6.2f}  max {v[-1]:6.2f}")
