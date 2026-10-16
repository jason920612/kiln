from p68_mounts import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
for typ in ("camel_husk", "camel"):
    clear_area()
    sv(f"tp {ME} 20.5 200 10.5", show=False)
    time.sleep(1.0)
    sv(f"summon minecraft:{typ} 20.5 200 12.5 {{PersistenceRequired:1b,NoAI:1b}}", show=False)
    time.sleep(1.2)
    sv(f"item replace entity @e[type={typ},limit=1] saddle with minecraft:saddle", show=False)
    sv(f"ride {ME} mount @e[type={typ},limit=1]", show=False)
    time.sleep(1.5)
    F(typ, "riding", a.state(0)["vehicle"])
    for hold in (0.3, 0.8, 1.5, 0.8, 0.3):
        eid = eid_of(typ)
        g0 = [e for e in ents_of(a, typ, 60) if e["id"] == eid][0]
        sv(f"tp @e[type={typ}] 20.5 200 10.5", show=False); time.sleep(1.2)
        g0 = [e for e in ents_of(a, typ, 60) if e["id"] == eid][0]
        a.look(0, 0)
        a.key("space", "down")
        time.sleep(hold)
        a.key("space", "up")
        time.sleep(2.0)
        g1 = [e for e in ents_of(a, typ, 60) if e["id"] == eid][0]
        F(typ, "hold", hold, "dash moved", round(math.hypot(g1["x"] - g0["x"], g1["z"] - g0["z"]), 2), "dy", round(g1["y"] - g0["y"], 2))
        time.sleep(3.5)  # cooldown
