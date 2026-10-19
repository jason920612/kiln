"""Compare ~/wp84/matrix_K.jsonl with matrix_V.jsonl (p84_matrix.py): print the mob/item pairs where the two servers differ in click success, inventory change, changed NBT keys
or effects (sounds / particles / events)."""
import json, os, sys
from pathlib import Path
H = Path.home() / "wp84"
def load(t):
    d = {}
    for l in (H / f"matrix_{t}.jsonl").read_text().splitlines():
        r = json.loads(l)
        d[(r["mob"], r["item"])] = r
    return d
K, V = load("K"), load("V")
n_same = n_diff = 0
for key in sorted(set(K) & set(V)):
    k, v = K[key], V[key]
    if "err" in k or "err" in v:
        if k.get("err") != v.get("err"):
            print(key, "err", k.get("err"), v.get("err"))
        continue
    if not (k["ok"] and v["ok"]):
        if k["ok"] != v["ok"]:
            print(f"{key[0]:16s} {key[1]:18s} click K {k['ok']} V {v['ok']}")
        continue
    diffs = []
    if k["inv"] != v["inv"]:
        diffs.append(f"inv K {k['inv']} V {v['inv']}")
    if k["changed"] != v["changed"]:
        diffs.append(f"nbt K {k['changed']} V {v['changed']}")
    fk, fv = set(k["fx"]), set(v["fx"])
    if fk != fv:
        diffs.append(f"fx onlyK {sorted(fk - fv)} onlyV {sorted(fv - fk)}")
    if diffs:
        n_diff += 1
        print(f"{key[0]:16s} {key[1]:18s} " + " | ".join(diffs))
    else:
        n_same += 1
print(f"same {n_same}, different {n_diff}", file=sys.stderr)
