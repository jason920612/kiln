"""wp84: compare the facts and packet recordings of a scenario run on both servers (par.sh p84_xxx.py): ~/wp84/facts/<scn>_K.txt vs _V.txt (line by line) and
~/wp84/spy/<scn>-<tag>_K.json vs _V.json (packet lines with numbers normalised).
usage: python3 p84_cmp.py scenario [-q]   (-q: only the lines that differ)"""
import json, sys
from pathlib import Path
H = Path.home() / "wp84"
scn = sys.argv[1]
quiet = "-q" in sys.argv
try:
    k = (H / "facts" / f"{scn}_K.txt").read_text().splitlines()
    v = (H / "facts" / f"{scn}_V.txt").read_text().splitlines()
except FileNotFoundError as e:
    k = v = []
    print("no facts:", e)
print(f"== facts {scn}: K {len(k)} lines, V {len(v)} lines")
for i in range(max(len(k), len(v))):
    a = k[i] if i < len(k) else "-"
    b = v[i] if i < len(v) else "-"
    if a == b:
        if not quiet:
            print("  SAME ", a[:230])
    else:
        print(f"  DIFF K {a[:330]}\n       V {b[:330]}")
print(f"== packets {scn}")
tags = sorted({p.name.rsplit("_", 1)[0] for p in (H / "spy").glob(f"{scn}-*_K.json")} | {p.name.rsplit("_", 1)[0] for p in (H / "spy").glob(f"{scn}-*_V.json")})
for tg in tags:
    try:
        kk = json.loads((H / "spy" / f"{tg}_K.json").read_text())
        vv = json.loads((H / "spy" / f"{tg}_V.json").read_text())
    except FileNotFoundError:
        print(f"  {tg}: only one side")
        continue
    out = []
    for key in sorted(set(kk) | set(vv)):
        if kk.get(key, 0) != vv.get(key, 0) and not key.startswith(("Pos", "PosRot", "Rot", "PlayerPosition", "SystemChat", "Time", "SetTime", "update_time", "entity_data", "attributes")):
            out.append(f"    K{kk.get(key, 0):>4} V{vv.get(key, 0):>4}  {key[:170]}")
    print(f"  {tg}: {sum(kk.values())} / {sum(vv.values())} packets, {len(out)} differing kinds")
    for o in out[:25]:
        print(o)
