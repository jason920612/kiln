from p68_small import *
reset()
sv("fill 10 200 10 30 240 30 minecraft:air", show=False)
give(ME, "minecraft:diamond_block", 1)
for what in ("tnt", "wind", "tnt_moving"):
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    rec_start(a)
    if what == "wind":
        give(ME, "minecraft:wind_charge", 2)
        a.select("minecraft:wind_charge")
        a.look(0, 90)
        time.sleep(0.4)
        a.mouse("right", "tap")
    elif what == "tnt":
        sv("summon minecraft:tnt 20.5 200.0 20.5 {fuse:2}", show=False, wait=0.0)
    else:
        a.look(0, 0)
        a.key("w", "down")
        time.sleep(0.5)
        sv("summon minecraft:tnt 20.5 200.0 22.0 {fuse:2}", show=False, wait=0.0)
    ys = []
    t0 = time.time()
    while time.time() - t0 < 2.0:
        time.sleep(0.05)
        s = a.state(0)
        ys.append((round(s["y"], 2), round(s["z"], 2)))
    a.key("w", "up")
    ls = json.loads(a.cmd("spylog clear"))
    print(what, "peak y", max(y for y, _ in ys), "end z", ys[-1][1], ys[:10], [re.sub(r"vol=\S+|pitch=\S+|#", "", l)[:48] for l in ls if "Move" not in l and "step" not in l and "Rot" not in l][:9])
    time.sleep(1.0)
