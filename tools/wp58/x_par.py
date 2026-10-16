from p68_mounts import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv("time set day", show=False)
for typ in ("parched", "skeleton", "bogged", "husk", "stray"):
    clear_area()
    sv(f"tp {ME} 20.5 200 8.5", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    time.sleep(1.0)
    sv(f"summon minecraft:{typ} 20.5 200 28.5 {{PersistenceRequired:0b}}", show=False)
    time.sleep(1.0)
    log = []
    for i in range(12):
        time.sleep(2.0)
        es = ents_of(a, typ, 60)
        n = sv(f"execute if entity @e[type={typ}]", show=False)
        log.append((i * 2 + 2, es and (round(es[0]["x"], 1), round(es[0]["z"], 1), es[0]["hp"]), "pass" in n and "fail" not in n))
    F(typ, "alive over 24 s:", log)
