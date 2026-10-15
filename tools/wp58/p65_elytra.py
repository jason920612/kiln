"""wp65: elytra + firework rockets: both players glide for a few minutes over a fresh area while the server's mspt and chunk counters
are logged; usage: p65_elytra.py DIM X Z DX DZ SECONDS  (start at (X, 120, Z), fly in direction (DX, DZ))"""
from p65_b2 import *

SY = int(os.environ.get("SY") or "120")


def fly(dimn, x0, z0, dx, dz, secs):
    prep()
    E = "minecraft:" + dimn
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"item replace entity {nm} armor.chest with minecraft:elytra", show=False)
        sv(f"give {nm} minecraft:firework_rocket[fireworks={{flight_duration:3}}] 64", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
    # the start platform (the End outside the island is void)
    sv(f"execute in {E} run forceload add {x0-8} {z0-8} {x0+8} {z0+8}", show=False)
    time.sleep(2)
    for i in range(20):
        out = sv(f"execute in {E} run setblock {x0} {SY-1} {z0} minecraft:stone", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out: break
        time.sleep(1.5)
    sv(f"execute in {E} run fill {x0-3} {SY-1} {z0-3} {x0+3} {SY-1} {z0+3} minecraft:stone", show=False)
    for i, nm in enumerate(("KilnTest", "Obs")):
        sv(f"execute in {E} run tp {nm} {x0 + 0.5 + i} {SY} {z0 + 0.5}", show=False)
    time.sleep(6)
    n = math.hypot(dx, dz); ux, uz = dx / n, dz / n
    yaw = math.degrees(math.atan2(-ux, uz))
    for c in (a, b):
        c.look(yaw, 5)
        c.select("minecraft:firework_rocket")
    # jump off and open the wings (jump again in the air), then rocket
    for c in (a, b):
        c.key("space", "tap"); time.sleep(0.5); c.key("space", "tap"); time.sleep(0.3)
    time.sleep(1.0)
    F("gliding", a.state(0)["fallFlying"], b.state(0)["fallFlying"], pos(a))
    t0 = time.time(); last = t0; lastp = pos(a); log = []
    ch0 = chunks()
    rockets = 0
    while time.time() - t0 < secs:
        for c in (a, b):
            st = c.state(0)
            if st["health"] <= 0:
                continue
            if not st["fallFlying"]:
                # re-open the wings
                c.key("space", "tap"); time.sleep(0.3); c.key("space", "tap")
            pitch = -12
            if st["y"] < SY - 30: pitch = -35
            elif st["y"] > SY + 40: pitch = 15
            c.look(yaw, pitch)
        if int((time.time() - t0) * 2) % 4 == 0:
            for c in (a, b):
                c.mouse("right", "tap")
            rockets += 1
        time.sleep(0.5)
        if time.time() - last >= 10:
            p = pos(a)
            sp = math.hypot(p[0] - lastp[0], p[2] - lastp[2]) / (time.time() - last)
            log.append((round(time.time() - t0), p, round(sp, 1)))
            F("t+%d A" % (time.time() - t0), p, "speed %.1f b/s" % sp, "B", pos(b), "|", mspt()[:95])
            if time.time() - t0 > 20 and not os.path.exists("/tmp/_shot_taken"):
                shot("elytra_flight_" + dimn)
                open("/tmp/_shot_taken", "w").close()
            last = time.time(); lastp = p
    F("rockets", rockets, "final A", pos(a), "B", pos(b))
    F("chunks before", ch0[:200])
    F("chunks after", chunks()[:300])
    shot("elytra_" + dimn)

if __name__ == "__main__":
    fly(sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5]), int(sys.argv[6]))
