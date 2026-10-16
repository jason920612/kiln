"""wp71: compare two counter files (W71Hook on the vanilla server, FARM_STATS of farm69.rs on Kiln): cmp_stats.py A.stats B.stats [more...]
Several files on a side (A1,A2 vs B1,B2: `--` separates the sides) are summed. Prints, per (phase, class) the share of villager-ticks, and the tokens, behaviour starts, speed bins and
distances to the zombie whose per-tick rates differ most (a rate is a count over the villager-ticks of the same phase and class)."""
import sys
from collections import defaultdict
from math import sqrt


def load(paths):
    d = defaultdict(int)
    for p in paths:
        for l in open(p):
            k, v = l.rstrip("\n").split("\t")
            d[k] += int(v)
    return d


def main():
    args = sys.argv[1:]
    if "--" in args:
        i = args.index("--")
        a, b = load(args[:i]), load(args[i + 1:])
    else:
        a, b = load(args[:1]), load(args[1:2])
    ta = {k.split("|", 1)[1]: v for k, v in a.items() if k.startswith("ticks|")}
    tb = {k.split("|", 1)[1]: v for k, v in b.items() if k.startswith("ticks|")}
    print("golems", a.get("golem"), b.get("golem"), " opportunities with a panicking villager:",
          a.get("opp|any", 0) / max(1, a.get("opp|all", 1)), b.get("opp|any", 0) / max(1, b.get("opp|all", 1)), " (", a.get("opp|all"), b.get("opp|all"), ")")
    na, nb = sum(ta.values()), sum(tb.values())
    print("villager-ticks A %d B %d" % (na, nb))
    for key in sorted(set(ta) | set(tb)):
        print("  ticks %-6s A %.4f  B %.4f" % (key, ta.get(key, 0) / na, tb.get(key, 0) / nb))
    rows = []
    for kind in ("tok", "start"):
        for k in set(x for x in a if x.startswith(kind + "|")) | set(x for x in b if x.startswith(kind + "|")):
            ph, cls = k.split("|")[1:3]
            xa, xb = a.get(k, 0), b.get(k, 0)
            da, db = ta.get(ph + "|" + cls, 0), tb.get(ph + "|" + cls, 0)
            if da < 20000 or db < 20000:
                continue
            ra, rb = xa / da, xb / db
            if xa + xb < 200:
                continue
            z = (xa / da - xb / db) / sqrt(max(1e-12, (xa + xb) / (da + db) * (1 / da + 1 / db)))
            rows.append((abs(ra - rb) / max(ra, rb, 1e-9), z, k, ra, rb, xa, xb))
    rows.sort(reverse=True)
    print("\nlargest relative differences of token / start rates (per villager-tick), counts >= 200:")
    for rel, z, k, ra, rb, xa, xb in rows[:45]:
        print("  %-62s A %.5f B %.5f  rel %.2f  z %.1f  (n %d/%d)" % (k, ra, rb, rel, z, xa, xb))
    # distributions: speed and zombie distance
    for name, prefix in (("speed", "speed|"), ("zd", "zd|")):
        groups = defaultdict(dict)
        for k in set(x for x in a if x.startswith(prefix)) | set(x for x in b if x.startswith(prefix)):
            parts = k.split("|")
            g = "|".join(parts[1:-1])
            groups[g][int(parts[-1])] = (a.get(k, 0), b.get(k, 0))
        for g in sorted(groups):
            sa = sum(v[0] for v in groups[g].values())
            sb = sum(v[1] for v in groups[g].values())
            if sa < 20000 or sb < 20000:
                continue
            cum = lambda i: (sum(v[0] for j, v in groups[g].items() if j <= i) / sa, sum(v[1] for j, v in groups[g].items() if j <= i) / sb)
            if name == "speed":
                pts = [0, 1, 5, 10, 15, 20, 25, 30, 40, 50]
            else:
                pts = [2, 4, 6, 8, 10, 12, 14, 16, 20, 30, 48]
            print("\n%s %s cumulative share <= bin:" % (name, g))
            print("   bin   " + " ".join("%6d" % p for p in pts))
            print("   A     " + " ".join("%6.3f" % cum(p)[0] for p in pts))
            print("   B     " + " ".join("%6.3f" % cum(p)[1] for p in pts))
    # the grid: which cells, per (phase, class)
    cells = defaultdict(dict)
    for k, v in a.items():
        if k.startswith("grid|"):
            _, ph, cls, dx, dz = k.split("|")
            cells[(ph, cls)][(int(dx), int(dz))] = [v, 0]
    for k, v in b.items():
        if k.startswith("grid|"):
            _, ph, cls, dx, dz = k.split("|")
            cells[(ph, cls)].setdefault((int(dx), int(dz)), [0, 0])[1] = v
    for (ph, cls), g in sorted(cells.items()):
        sa, sb = sum(v[0] for v in g.values()), sum(v[1] for v in g.values())
        if sa < 50000 or sb < 50000:
            continue
        print("\ngrid %s %s: share of the villager-ticks by x (columns 1..11 = blocks 0..10) A / B" % (ph, cls))
        for label, i in (("A", 0), ("B", 1)):
            tot = sa if i == 0 else sb
            print("   " + label + " " + " ".join("%5.3f" % (sum(v[i] for (dx, dz), v in g.items() if dx == x) / tot) for x in range(0, 13)))
        print("   by z (rows 1..9 = blocks 0..8) A / B")
        for label, i in (("A", 0), ("B", 1)):
            tot = sa if i == 0 else sb
            print("   " + label + " " + " ".join("%5.3f" % (sum(v[i] for (dx, dz), v in g.items() if dz == z) / tot) for z in range(0, 11)))


main()
