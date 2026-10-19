"""wp82 item 5: when does a non-player entity change level? A cow in a nether portal frame (frozen game, `tick step 1` at a time): after every step
which level it is in and its position (and motion). Kiln against vanilla. usage: portal82.py [steps] [mob]"""
from lab import *
STEPS = int(sys.argv[1]) if len(sys.argv) > 1 else 8
MOB = sys.argv[2] if len(sys.argv) > 2 else "cow"
setup(frozen=False, size=60)
for t in "KV":
    w(t, "execute in minecraft:the_nether run forceload add 0 0 60 60")
time.sleep(4)
import os
NOAI = os.environ.get("NOAI", "1")
X, Y, Z = int(os.environ.get("PX", "20")), int(os.environ.get("PY", "150")), int(os.environ.get("PZ", "20"))

def where(t):
    out = []
    for d in ("overworld", "the_nether"):
        o = ask(t, f"execute as @e[tag=c82] at @s if dimension minecraft:{d} run data get entity @s Pos", wait=0.25)
        m = re.search(r"\[(-?[\d.]+)d, (-?[\d.]+)d, (-?[\d.]+)d\]", o)
        if m:
            out.append((d[:3], tuple(round(float(g), 3) for g in m.groups())))
    return out

def frame(dim):
    # obsidian frame 4 wide 5 tall in the x axis, portal blocks inside, a stone floor
    ex = f"execute in minecraft:{dim} run "
    both_(ex + f"fill {X - 3} {Y - 2} {Z - 3} {X + 6} {Y + 8} {Z + 3} minecraft:air")
    both_(ex + f"fill {X - 2} {Y - 1} {Z - 2} {X + 5} {Y - 1} {Z + 2} minecraft:stone")
    both_(ex + f"fill {X} {Y} {Z} {X + 3} {Y + 4} {Z} minecraft:obsidian")
    both_(ex + f"fill {X + 1} {Y + 1} {Z} {X + 2} {Y + 3} {Z} minecraft:nether_portal[axis=x]")

for src in ("overworld", "the_nether"):
    for t in "KV":
        w(t, "tick unfreeze"); w(t, "kill @e[tag=c82]")
    time.sleep(1.5)
    frame(src)
    if src == "the_nether" and os.environ.get("FLAT"):
        # a bare destination: a flat stone floor at y 69 and air above, around the scaled entry point
        tx, tz = int((X + 1.5) * 8), int(Z * 8 + 4)
        both_(f"execute in minecraft:overworld run forceload add {tx - 24} {tz - 24} {tx + 24} {tz + 24}")
        time.sleep(12)
        both_(f"execute in minecraft:overworld run fill {tx - 24} 66 {tz - 24} {tx + 24} 66 {tz + 24} minecraft:stone")
        both_(f"execute in minecraft:overworld run fill {tx - 24} 67 {tz - 24} {tx + 24} 110 {tz + 24} minecraft:air")
        time.sleep(2)
    time.sleep(1)
    # frozen first, then the mob: it must not tick before the first step (Kiln runs the console one tick later than vanilla)
    both_("tick freeze")
    time.sleep(1.0)
    both_(f"execute in minecraft:{src} run summon minecraft:{MOB} {X + 1}.5 {Y + 1} {Z}.5 {{Tags:[\"c82\"],NoAI:{NOAI}b,PersistenceRequired:1b}}")
    time.sleep(1.0)
    F("==", src, "-> the other level; after every step:")
    rows = {"K": [], "V": []}
    for i in range(STEPS):
        step(1)
        for t in "KV":
            rows[t].append(where(t))
    for i in range(STEPS):
        F(f"  step {i + 1}: K {rows['K'][i]}   V {rows['V'][i]}")
    both_("tick unfreeze")
