from p68_small import *
reset()
sv("fill 10 200 10 30 240 30 minecraft:air", show=False)
sv(f"tp {ME} 20.5 200 20.5", show=False)
time.sleep(1.0)
give(ME, "minecraft:wind_charge", 6)
a.select("minecraft:wind_charge")
for k in range(2):
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.2)
    a.look(0, 90)
    time.sleep(0.4)
    rec_start(a)
    a.mouse("right", "tap")
    ys = []
    t0 = time.time()
    while time.time() - t0 < 2.2:
        time.sleep(0.05)
        ys.append(round(a.state(0)["y"], 2))
    ls = json.loads(a.cmd("spylog clear"))
    print(k, "max", max(ys), "ys", ys[:14])
    for l in ls:
        if "Move" not in l and "step" not in l and "Rot" not in l:
            print("   ", l[:230])
