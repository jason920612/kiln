"""wp65: poses and states of A as B's client shows them: swimming, crawling under a 1-high gap, climbing, creative flying, sleeping, riding a boat;
Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p65_b2 import *


def poses():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
        sv(f"gamemode survival {nm}", show=False)
    sv("fill -20 198 -20 20 199 20 minecraft:stone", show=False)
    sv("fill -20 200 -20 20 215 20 minecraft:air", show=False)
    sv("fill 4 199 -3 10 199 3 minecraft:water", show=False)
    sv("fill 4 198 -3 10 198 3 minecraft:stone", show=False)
    # a 1-high tunnel with stone around: crawl
    sv("fill -10 200 -10 -4 202 -10 minecraft:stone", show=False)
    sv("fill -10 200 -9 -4 200 -9 minecraft:stone", show=False)
    sv("fill -10 201 -9 -4 201 -9 minecraft:air", show=False)
    sv("fill -10 202 -9 -4 202 -9 minecraft:stone", show=False)
    sv("tp KilnTest 0.5 200 0.5", show=False)
    sv("tp Obs 0.5 200 4.5", show=False)
    time.sleep(2.5)
    pa = eid_of(b)
    b.lookat(0.5, 201, 0.5)
    # swim: A in the pool, sprints forward
    sv("tp KilnTest 5.5 200 0.5", show=False)
    time.sleep(1.5)
    a.look(-90, 20)
    a.key("w", "down"); a.key("ctrl", "down")
    time.sleep(1.5)
    sv("tp Obs 3.5 200 0.5", show=False)
    time.sleep(1.0)
    pa = eid_of(b)
    e = ent(b, pa) if pa else {}
    F("A sprint-swimming in the pool: B sees pose", e.get("pose"), "sprint", e.get("sprint"), "y", round(e.get("y", 0), 2))
    a.key("w", "up"); a.key("ctrl", "up")
    # crawl: A walks into the 1-high gap
    sv("tp KilnTest -11 200 -8.5", show=False)
    time.sleep(1.5)
    a.look(-90, 0)
    a.key("w", "down")
    sv("tp Obs -6.5 201 -8.5", show=False)
    time.sleep(2.0)
    pa = eid_of(b)
    e = ent(b, pa) if pa else {}
    F("A moving into a 1-high gap: B sees pose", e.get("pose"), "x", round(e.get("x", 0), 1))
    a.key("w", "up")
    # creative flight
    sv("tp KilnTest 0.5 200 0.5", show=False)
    sv("tp Obs 0.5 200 4.5", show=False)
    sv("gamemode creative KilnTest", show=False)
    time.sleep(2)
    a.key("space", "tap"); time.sleep(0.12); a.key("space", "tap"); time.sleep(0.5)
    a.key("space", "down"); time.sleep(1.2); a.key("space", "up")
    time.sleep(0.8)
    st = a.state(0)
    pa = eid_of(b)
    e = ent(b, pa) if pa else {}
    F("A creative flying: client y", round(st["y"], 1), "onGround", st["onGround"], "| B sees y", round(e.get("y", 0), 1), "pose", e.get("pose"))
    # stop flying (double tap) and drop
    a.key("space", "tap"); time.sleep(0.12); a.key("space", "tap")
    time.sleep(2.0)
    F("A after toggling off: y", round(a.state(0)["y"], 1), "onGround", a.state(0)["onGround"])
    sv("gamemode survival KilnTest", show=False)
    # climbing: ladder
    sv("fill 12 200 0 12 205 0 minecraft:ladder[facing=west]", show=False)
    sv("fill 13 200 0 13 205 0 minecraft:stone", show=False)
    sv("tp KilnTest 11.5 200 0.5", show=False)
    time.sleep(1.5)
    a.look(-90, -30)
    a.key("w", "down"); time.sleep(2.0)
    pa = eid_of(b)
    e = ent(b, pa) if pa else {}
    F("A on a ladder: client y", round(a.state(0)["y"], 1), "B sees y", round(e.get("y", 0), 1), "pose", e.get("pose"))
    a.key("w", "up")
    shot("poses")


if __name__ == "__main__":
    poses()
