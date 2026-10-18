"""wp76: the final table from finalbench's JSON (DIR/K_base.json, K_new.json, V.json): mean of the 100-tick averages and
the worst tick seen (`/tick query` P99 of 100 = the slowest) per scene, item counts at the end.
python3 table76.py DIR
"""
import json
import sys

dir_ = sys.argv[1]
d = {k: {r["scene"]: r for r in json.load(open(f"{dir_}/{k}.json"))} for k in ("K_base", "K_new", "V")}
skip = ("idle", "scatter idle", "hut idle (killer on)", "lavafall idle")
print("| 場景 | 物品(Kiln 改後 / 改前 / 原版) | Kiln 改前 | Kiln 改後 | 原版 26.3 |")
print("| --- | --- | --- | --- | --- |")


def cell(k, s):
    r = d[k].get(s)
    return f"{r['mean']:.2f} / {r['p99max']:.0f}" if r else "-"


for s in d["K_new"]:
    if s.startswith("hazard") or s in skip:
        continue
    items = "/".join(str(d[k].get(s, {}).get("items1", "-")) for k in ("K_new", "K_base", "V"))
    print(f"| {s} | {items} | {cell('K_base', s)} | {cell('K_new', s)} | {cell('V', s)} |")
