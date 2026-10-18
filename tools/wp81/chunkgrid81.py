"""wp81: the monster spawn attempts per chunk (the `chunkAttempt MONSTER cx,cz` counters of both servers, a box covering the whole area) as two grids:
which chunks Kiln (K) and vanilla (V) attempt, and how often. usage: chunkgrid79.py [K_FILE V_FILE]"""
import os, re, sys
HOME = os.path.expanduser("~")
kf = sys.argv[1] if len(sys.argv) > 1 else HOME + "/wp81/K/counters.out"
vf = sys.argv[2] if len(sys.argv) > 2 else HOME + "/wp81/V/agent.out"


def read(p):
    d = {}
    for line in open(p):
        k, _, v = line.rstrip("\n").rpartition("\t")
        m = re.match(r"chunkAttempt MONSTER (-?\d+),(-?\d+)$", k)
        if m:
            d[(int(m.group(1)), int(m.group(2)))] = int(v)
    return d


K, V = read(kf), read(vf)
xs = sorted({c[0] for c in K} | {c[0] for c in V})
zs = sorted({c[1] for c in K} | {c[1] for c in V})
print("chunks attempted: K", len(K), "V", len(V), " only K:", len(set(K) - set(V)), " only V:", len(set(V) - set(K)))
print("rows z, columns x; K/V attempts (. = none)")
print("      " + " ".join("%11d" % x for x in xs))
for z in zs:
    print("%5d " % z + " ".join(("%5s/%-5s" % (K.get((x, z), "."), V.get((x, z), "."))) for x in xs))
