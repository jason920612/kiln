"""wp68 #6: compare the dumps of p68_persist.py: what a restart changed on each server, and where Kiln and vanilla differ."""
import json, re, sys, difflib
from pathlib import Path
D = Path.home() / "wp68" / "persist"
ld = lambda tag, t: json.loads((D / f"{tag}_{t}.json").read_text())
b = {t: ld("before", t) for t in "KV"}
a = {t: ld("after", t) for t in "KV"}

def norm(s):
    s = s.replace("\n", "\n")
    s = re.sub(r"^(INFO\s+)?\S*\s*", "", s) if False else s
    s = re.sub(r"System chat: ", "", s)
    s = re.sub(r"INFO kiln_sim::commands: ", "", s)
    return s

def toks(s):
    # split compound-ish text into comparable tokens
    s = norm(s)
    return [t.strip() for t in re.split(r",\s(?=[A-Za-z_\"]+: )|(?<=\{)|(?=\})|\n", s) if t.strip()]

def changed(t, k):
    x, y = toks(b[t].get(k, "")), toks(a[t].get(k, ""))
    return [d for d in difflib.unified_diff(x, y, lineterm="", n=0) if d[:1] in "+-" and d[:3] not in ("+++", "---")]

keys = sorted(set(b["K"]) | set(b["V"]))
print("keys", len(keys))
for k in keys:
    if k in ("tick_info",):
        continue
    ck, cv = changed("K", k), changed("V", k)
    if k == "clock_lamp_samples":
        print(k, "K", b["K"][k], a["K"][k], "V", b["V"][k], a["V"][k]); continue
    if k == "client":
        print(k, "K", a["K"][k][:200], "V", a["V"][k][:200]); continue
    # tokens that differ in the restart delta between K and V
    sk = {re.sub(r"-?\d+(\.\d+)?[dfLsb]?", "N", d) for d in ck}
    sv_ = {re.sub(r"-?\d+(\.\d+)?[dfLsb]?", "N", d) for d in cv}
    if sk != sv_ or "-v" in sys.argv:
        print("==", k)
        print("   K changed:", [d[:150] for d in ck][:12])
        print("   V changed:", [d[:150] for d in cv][:12])
