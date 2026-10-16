from p68_creaking import *
build()
sv("kill @e[type=creaking]", show=False)
sv("setblock 20 202 24 minecraft:creaking_heart[axis=y,natural=true]", show=False)
time.sleep(0.5)
sv("kill @e[type=creaking]", show=False)
for k, (cx, cz) in enumerate(((10.5, 22.5), (26.5, 28.5))):
    sv("kill @e[type=creaking]", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1)
    sv(f"summon minecraft:creaking {cx} 200 {cz} {{home_pos:[I;20,202,24],home_radius:32}}", show=False)
    time.sleep(0.8)
    a.look(0, 0)
    tr = []
    for _ in range(24):
        c = ents_of(a, "creaking", 40)
        if c:
            tr.append((round(c[0]["x"], 1), round(c[0]["z"], 1)))
        time.sleep(0.5)
    print(k, "away:", tr[::2])
