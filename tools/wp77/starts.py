#!/usr/bin/env python3
"""Per start (S line) statistics of the valid-position traces of the two servers: K has the S line before its rows, V after (logged at return)."""
import collections
import sys


def blocks(path, s_first):
    out, rows = [], []
    for line in open(path):
        w = line.split()
        if not w:
            continue
        if w[0] == "S":
            if s_first:
                if rows or out:
                    out[-1][1].extend(rows) if out else None
                out.append(((int(w[1]), int(w[2]), int(w[3])), []))
                rows = []
            else:
                out.append(((int(w[1]), int(w[2]), int(w[3])), rows))
                rows = []
        else:
            r = (w[1], int(w[2]), int(w[3]), int(w[4]), w[5] == "true")
            if s_first:
                if out:
                    out[-1][1].append(r)
            else:
                rows.append(r)
    return out


for name, path, s_first in (("K", sys.argv[1], True), ("V", sys.argv[2], False)):
    bl = blocks(path, s_first)
    if len(sys.argv) > 3:
        bx = [int(v) for v in sys.argv[3].split(",")]
        bl = [(s, r) for s, r in bl if bx[0] <= s[0] <= bx[3] and bx[2] <= s[2] <= bx[5] and s[1] >= 120]
    by_y = collections.defaultdict(list)
    for s, rows in bl:
        by_y[s[1]].append(rows)
    print(name, "starts", len(bl))
    for y in sorted(by_y):
        starts = by_y[y]
        n = len(starts)
        rows = sum(len(r) for r in starts)
        true = sum(1 for r in starts for x in r if x[4])
        empty = sum(1 for r in starts if not r)
        sizes = collections.Counter(len(r) for r in starts)
        print(f"  y={y}: starts {n} rows/start {rows / n:.2f} true/start {true / n:.2f} empty {empty} sizes {dict(sorted(sizes.items()))}")
    # y=129: how many true rows per start, and in which position of the start
    ys = by_y.get(129, [])
    seq = collections.Counter(tuple(int(x[4]) for x in r) for r in ys)
    for k, c in seq.most_common(12):
        print("   y129", "".join(map(str, k)), c)
