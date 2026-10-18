"""wp76: inclusive time below a function in a frame-pointer perf profile.

perf script -i DATA -F ip,sym --no-inline | python3 tree76.py ROOT [DEPTH] [MIN%]
  ROOT: a substring of the function to look below (its first frame from the top of each stack that matches);
  prints the call tree under it (callees, by share of all samples), DEPTH levels deep, branches over MIN% only.
"""
import re
import sys
from collections import defaultdict

root = sys.argv[1]
depth = int(sys.argv[2]) if len(sys.argv) > 2 else 4
min_pct = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5


def clean(sym):
    sym = re.sub(r"<[^<>]*>", "", sym)
    sym = re.sub(r"<[^<>]*>", "", sym)
    sym = re.sub(r"\+0x[0-9a-f]+$", "", sym)
    return sym.strip()


total = 0
tree = defaultdict(int)
stack = []


def flush():
    global total
    if not stack:
        return
    total += 1
    frames = [clean(s) for s in stack]  # leaf first
    frames.reverse()  # root first
    for i, f in enumerate(frames):
        if root in f:
            path = tuple(frames[i:i + depth + 1])
            for k in range(1, len(path) + 1):
                tree[path[:k]] += 1
            break


for line in sys.stdin:
    line = line.rstrip("\n")
    if not line.strip():
        flush()
        stack = []
        continue
    parts = line.strip().split(None, 1)
    if len(parts) == 2 and re.fullmatch(r"[0-9a-f]+", parts[0]):
        stack.append(parts[1].rsplit(" (", 1)[0])
flush()


def show(prefix):
    kids = sorted(((n, p) for p, n in tree.items() if len(p) == len(prefix) + 1 and p[:len(prefix)] == prefix), reverse=True)
    for n, p in kids:
        pct = 100.0 * n / max(total, 1)
        if pct < min_pct:
            continue
        print(f"{'  ' * len(prefix)}{pct:6.2f}%  {p[-1][:150]}")
        show(p)


print(f"{total} samples")
show(())
