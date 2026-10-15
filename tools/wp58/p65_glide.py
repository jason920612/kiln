"""wp65: glide and land; what does the client (and the observer) think afterwards? Kiln vs vanilla (TARGET)."""
from p65_b2 import *
import p64
from p64 import tidy, spylog_ts
p61.VDIR = HOME / "wp65" / "vanilla"  # (importing p64 pointed it at wp64)

def glide():
    prep()
    E = "minecraft:overworld"
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect clear {nm} minecraft:slow_falling", show=False)
        sv(f"item replace entity {nm} armor.chest with minecraft:elytra", show=False)
        sv(f"gamemode survival {nm}", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv(f"execute in {E} run forceload add 0 0 0 0", show=False)
    # a flat stone floor at y=199 (forceloaded), start 40 above
    sv("fill -30 199 -30 30 199 30 minecraft:stone", show=False)
    sv("fill -30 200 -30 30 260 30 minecraft:air", show=False)
    sv("tp KilnTest 0.5 250 0.5", show=False); sv("tp Obs 4.5 250 0.5", show=False)
    time.sleep(3)
    a.cmd("spy on"); b.cmd("spy on")
    spylog_ts(a); spylog_ts(b)
    for c in (a, b):
        c.look(-90, 35)
        c.key("space", "tap"); time.sleep(0.4); c.key("space", "tap")
    t0 = time.time()
    seen = []
    while time.time() - t0 < 60:
        sa, sb = a.state(0), b.state(0)
        seen.append((round(time.time() - t0, 1), round(sa["x"], 1), round(sa["y"], 1), sa["fallFlying"], sa["onGround"], sb["fallFlying"]))
        if time.time() - t0 > 3 and sa["onGround"] and sb["onGround"]:
            break
        time.sleep(0.5)
    time.sleep(2)
    F("track", seen[::4])
    sa, sb = a.state(0), b.state(0)
    F("after landing: A client fallFlying", sa["fallFlying"], "onGround", sa["onGround"], "| B", sb["fallFlying"], sb["onGround"])
    pb = eid_of(a); pa = eid_of(b)
    F("A sees B glide", ent(a, pb)["glide"] if pb else "unseen", "| B sees A", ent(b, pa)["glide"] if pa else "unseen")
    o = tidy(spylog_ts(b), drop=p64.DROP, drop_re=r"^(sound|Sound)")
    s = tidy(spylog_ts(a), drop=p64.DROP)
    F("B's view of A's entity data:", [l for l in o if "entity_data" in l and "player" in l][-8:])
    F("A's own entity data:", [l for l in s if "entity_data" in l and " 0=" in l][-8:])

if __name__ == "__main__":
    glide()
