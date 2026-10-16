"""wp70: bench70 results as a markdown table: the worst tick (ms) and the cost per block (µs, worst minus idle) of each case,
the mean of the runs given per side.

usage: python3 table70.py base=final_base_1.json,final_base_2.json new=... V=...
"""

import json
import sys
from pathlib import Path


def load(files):
    runs = [json.loads(Path(f).read_text()) for f in files]
    out = {}
    for run in runs:
        for r in run:
            out.setdefault(r["case"], []).append(r)
    return out


def main():
    sides = []
    for arg in sys.argv[1:]:
        name, files = arg.split("=", 1)
        sides.append((name, load(files.split(","))))
    cases = list(sides[0][1])
    head = "| case | blocks | " + " | ".join(f"{n} worst ms | {n} µs/block" for n, _ in sides) + " |"
    print(head)
    print("|" + " --- |" * (2 + 2 * len(sides)))
    for c in cases:
        cells = []
        blocks = None
        for _, d in sides:
            rs = d.get(c, [])
            if not rs:
                cells += ["", ""]
                continue
            blocks = rs[0]["blocks"]
            worst = sum(r["p99"] for r in rs) / len(rs)
            per = sum((r["p99"] - r["p50"]) * 1000 / max(r["blocks"], 1) for r in rs) / len(rs)
            cells += [f"{worst:.1f}", f"{per:.2f}" if blocks > 1 else "-"]
        print(f"| {c} | {blocks} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
