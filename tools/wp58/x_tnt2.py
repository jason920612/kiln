from p68_small import *
reset()
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
for k in range(2):
    sv("fill 10 199 10 30 199 30 minecraft:stone", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    rec_start(a)
    sv("summon minecraft:tnt 20.5 200.0 20.5 {fuse:2}", show=False, wait=0.0)
    ys = []
    t0 = time.time()
    while time.time() - t0 < 1.5:
        time.sleep(0.05)
        ys.append(round(a.state(0)["y"], 2))
    ls = json.loads(a.cmd("spylog clear"))
    print("peak y", max(ys), ys[:12])
    for l in ls:
        if "Move" not in l and "step" not in l and "Rot" not in l and "chunk" not in l.lower():
            print("   ", l[:400])
