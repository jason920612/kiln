"""K vs V of the same dump (before/after): keys whose values differ, as token differences."""
import json, re, sys, difflib
from pathlib import Path
D = Path.home() / "wp68" / "persist"
tag = sys.argv[1]
k = json.loads((D / f"{tag}_K.json").read_text()); v = json.loads((D / f"{tag}_V.json").read_text())
def toks(s):
    s = re.sub(r"System chat: |INFO kiln_sim::commands: ", "", s)
    s = re.sub(r"^.*?(has the following block data: |commands\.data\.block\.query\[\d+, \d+, \d+, |has the following entity data: |commands\.data\.entity\.query\[[^,]*, )", "", s, flags=re.S)
    return sorted(t.strip(" ,[]{}") for t in re.split(r",\s(?=[A-Za-z_\"]+: )|(?<=\{)|(?=\})|\n", s) if t.strip(" ,[]{}"))
for key in sorted(k):
    if key in ("clock_lamp_samples", "tick_info", "client"): continue
    a, b = toks(k[key]), toks(v.get(key, ""))
    d = [x for x in difflib.unified_diff(a, b, lineterm="", n=0) if x[:1] in "+-" and x[:3] not in ("+++", "---")]
    if d:
        print("==", key); [print("   ", x[:170]) for x in d[:14]]
