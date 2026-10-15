"""wp65: A glides (steep dive) and lands; timeline of A's client flag and of the entity-data packets A received."""
from p65_b2 import *
import p64, os
from p64 import spylog_ts
p61.VDIR = HOME / "wp65" / "vanilla"

def run():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect clear {nm} minecraft:slow_falling", show=False)
        if os.environ.get("EQUIP") == "use":
            sv(f"give {nm} minecraft:elytra 1", show=False)
        else:
            sv(f"item replace entity {nm} armor.chest with minecraft:elytra", show=False)
        sv(f"gamemode survival {nm}", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("fill -30 199 -30 30 199 30 minecraft:stone", show=False)
    sv("fill -30 200 -30 30 260 30 minecraft:air", show=False)
    alive(a)
    if os.environ.get("EQUIP") == "use":
        a.select("minecraft:elytra"); a.mouse("right", "tap"); time.sleep(1.0)
    sv("tp Obs 8.5 200 8.5", show=False)
    a.cmd("spy on")
    if os.environ.get("START") == "ground":
        sv("tp KilnTest 0.5 200 0.5", show=False)
        time.sleep(3)
        spylog_ts(a)
        a.look(-90, 8)
        t0 = time.time()
        a.key("space", "tap"); time.sleep(0.45); a.key("space", "tap")
    else:
        sv("tp KilnTest 0.5 330 0.5", show=False)
        for _ in range(100):
            if a.state(0)["y"] > 300:
                break
            time.sleep(0.05)
        spylog_ts(a)
        a.look(-90, 60)
        t0 = time.time()
        a.key("space", "tap"); time.sleep(0.25); a.key("space", "tap")
    tl = []; prev = None
    while time.time() - t0 < 25:
        s = a.state(0)
        cur = (s["fallFlying"], s["onGround"])
        if cur != prev:
            tl.append((round(time.time() - t0, 2), "client fallFlying=%s onGround=%s y=%.1f" % (cur + (s["y"],))))
            prev = cur
        time.sleep(0.05)
    for t, l in tl: F(t, l)
    lines = spylog_ts(a)
    base = None
    for l in lines:
        ms, _, text = l.partition(" ")
        if "entity_data" in text or "game_event" in text or "player_abilities" in text:
            F("pkt +%sms" % ms, text[:150]) if "1=" not in text else None
    s = a.state(0)
    F("final client fallFlying", s["fallFlying"], "onGround", s["onGround"], pos(a))
    F("elytra", a.cmd("comps inv 38")[:200])
    F("mspt", mspt())

if __name__ == "__main__":
    run()
