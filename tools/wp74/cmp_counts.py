"""chi-square of two count dicts (K, V) read from a RESULT json line: cmp_counts.py file [key1 key2]"""
import json, sys
s = open(sys.argv[1]).read()
line = [l for l in s.splitlines() if l.startswith("RESULT")][-1]
d = json.loads(line[line.index("{"):])
K, V = d[sys.argv[2] if len(sys.argv) > 3 else "K"], d[sys.argv[3] if len(sys.argv) > 3 else "V"]
nk, nv = sum(K.values()), sum(V.values())
chi = 0; df = 0
print("totals", nk, nv)
for k in K:
    a, b = K[k], V.get(k, 0)
    if a + b == 0: continue
    p = (a + b) / (nk + nv)
    ea, eb = p * nk, p * nv
    c = (a - ea) ** 2 / ea + (b - eb) ** 2 / eb
    chi += c; df += 1
    print(f"{k:18s} K{a:5d} V{b:5d}  chi {c:5.2f}")
print("chi2 %.1f df %d" % (chi, df - 1))
