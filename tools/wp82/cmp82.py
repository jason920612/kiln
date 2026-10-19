"""wp82: the spawner's decision counters of Kiln (K/counters.out) and vanilla (V/agent.out) side by side: cmp81.py [prefix ...]
Counts are per tick of spawning, so the two servers must have run the same number of ticks; the chi-square is per group of keys."""
import os, sys
HOME = os.path.expanduser("~")


def read(p):
    d = {}
    for line in open(p):
        k, _, v = line.rstrip("\n").rpartition("\t")
        d[k] = int(v)
    return d


K, V = read(HOME + "/wp82/K/counters.out"), read(HOME + "/wp82/V/agent.out")
prefixes = sys.argv[1:] or [""]
keys = sorted(k for k in set(K) | set(V) if any(k.startswith(p) for p in prefixes))
chi, df = 0.0, 0
nk, nv = sum(K.get(k, 0) for k in keys), sum(V.get(k, 0) for k in keys)
for k in keys:
    a, b = K.get(k, 0), V.get(k, 0)
    c = 0.0
    if a + b and nk and nv:
        p = (a + b) / (nk + nv)
        ea, eb = p * nk, p * nv
        c = (a - ea) ** 2 / ea + (b - eb) ** 2 / eb
        chi += c
        df += 1
    print(f"{k[:90]:90s} K{a:8d} V{b:8d}  chi {c:6.2f}")
print("totals", nk, nv, "chi2 %.1f df %d" % (chi, max(df - 1, 0)))
