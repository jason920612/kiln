"""wp66 item 2: ringing the bell makes the raiders within 48 blocks glow (BellBlockEntity: shaking 50 ticks, resonating 40 ticks, then
`Effects.GLOWING` 60 ticks to every raider within 48 blocks that is not too far). Setup: three pillagers (no AI) 8 / 30 / 60 blocks from the bell;
the actor rings the bell with a real click; the glowing of each is sampled every 0.5 s for 12 s. usage: TARGET=K|V [ACTOR..] python3 p66_bell.py OUT.json"""
import sys
from p66_raid import *

BX, BY, BZ = 1463, 111, -1043

if __name__ == "__main__":
    a = client(PORT)
    S("gamerule advance_time false", "time set 6000", "tick rate 20")
    S("kill @e[type=minecraft:pillager]")
    dists = (8, 30, 47, 60)
    for d in dists:
        S(f"summon minecraft:pillager {BX + d}.5 {BY} {BZ}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"bell{d}\"]}}")
    S(f"forceload add {BX} {BZ} {BX+70} {BZ}")
    time.sleep(3)
    # a floor bell is only rung by a hit on a side face along its facing axis: stand 4 blocks away on that axis, the eye a little above the bell's body
    axis = "z"
    for f in ("north", "south", "east", "west"):
        out = S(f"execute if block {BX} {BY} {BZ} minecraft:bell[facing={f}]")
        if any("pass" in l for l in out):
            axis = "z" if f in ("north", "south") else "x"
    F("bell facing axis", axis)
    if axis == "z":
        S(f"tp {NAME} {BX}.5 {BY} {BZ+4}.5")
    else:
        S(f"tp {NAME} {BX+4}.5 {BY} {BZ}.5")
    time.sleep(2)
    a.lookat(BX + 0.5, BY + 0.45, BZ + 0.5)
    time.sleep(1.5)
    h = a.state(0).get("hit")
    F("actor looks at", h)
    res = {d: [] for d in dists}
    t0 = time.time()
    a.mouse("right", "tap")
    rung = time.time()
    F("rung")
    while time.time() - rung < 12:
        for d in dists:
            out = S(f'execute as @e[type=minecraft:pillager,tag=bell{d}] run data get entity @s active_effects')
            glow = any("glowing" in l for l in out)
            res[d].append((round(time.time() - rung, 1), glow))
        time.sleep(0.3)
    for d in dists:
        on = [t for t, g in res[d] if g]
        F(f"pillager at {d} blocks: glowing from {on[0] if on else None} s to {on[-1] if on else None} s (samples {len(on)}/{len(res[d])})")
    S("kill @e[type=minecraft:pillager]")
    json.dump(res, open(sys.argv[1], "w"))
