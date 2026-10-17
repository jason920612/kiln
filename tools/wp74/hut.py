import sys
from lock import *
setup(frozen=False)
both_("gamemode creative KilnTest")
both_("execute in minecraft:overworld run tp KilnTest -1276 100 740")
time.sleep(12)
for t, c in clients().items():
    st = c.state(0)
    print(t, st.get("x"), st.get("y"), st.get("z"))
    r = c.find("planks", 20, 400)
    xs = [(p[0], p[1], p[2]) if isinstance(p, list) else (p["x"], p["y"], p["z"]) for p in (r if isinstance(r, list) else r.get("blocks", r))]
    print(t, len(xs), "min", [min(a[i] for a in xs) for i in range(3)] if xs else None, "max", [max(a[i] for a in xs) for i in range(3)] if xs else None)
