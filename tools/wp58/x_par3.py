from p68_mounts import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv("time set day", show=False)
for typ in ("parched", "skeleton"):
    clear_area()
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    time.sleep(1.0)
    sv(f"summon minecraft:{typ} 20.5 200 28.5 {{PersistenceRequired:1b}}", show=False)
    time.sleep(0.5)
    rec_start(a)
    log = []
    arrows = 0
    for i in range(30):
        time.sleep(0.5)
        es = ents_of(a, typ, 60)
        arrows = max(arrows, len([e for e in a.state(60)["entities"] if "arrow" in e["t"]]))
        if i % 4 == 3:
            e = es[0] if es else None
            log.append((i // 2 + 1, e and (round(e["x"], 1), round(e["z"], 1), e["hp"]), a.state(0)["health"], [f["id"].split(":")[1] for f in a.state(0)["effects"]]))
    F(typ, "max arrows", arrows, log)
    F(typ, "entdata", a.cmd(f"entdata {es[0]['id']}")[:80] if es else None, [l[:70] for l in lines() if "sound" in l and "step" not in l][:6])
