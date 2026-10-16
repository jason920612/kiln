from p68_trial import *
VAULT = (233, -28, 244)
sv(f"gamemode survival {ME}", show=False)
sv(f"tp {ME} 231.5 -28 244.5", show=False)
time.sleep(2.0)
F("vault block:", blk(a, *VAULT), sv("data get block 233 -28 244", show=False)[-700:])
give(ME, "minecraft:trial_key", 3)
a.select("minecraft:trial_key")
a.lookat(233.3, -27.5, 244.5)
time.sleep(0.8)
F("aim:", a.state(0).get("hit"))
for attempt in range(2):
    items = collections.Counter()
    rec_start(a)
    a.mouse("right", "tap")
    t0 = time.time()
    seen = set()
    states = []
    while time.time() - t0 < 12:
        time.sleep(0.4)
        st = blk(a, *VAULT)
        if not states or states[-1][1] != st:
            states.append((round(time.time() - t0, 1), st))
        for e in a.state(12)["entities"]:
            if e["t"] == "item" and e["id"] not in seen:
                seen.add(e["id"]); items[e.get("item")] += e.get("n", 1)
    ls = lines()
    F("attempt", attempt, "states:", states, "items:", dict(items), "keys left:", a.inv().get("minecraft:trial_key", 0), "sounds:", [l[:70] for l in ls if "sound" in l and "step" not in l][:6])
    a.collect(6, 4)
F("vault data after:", sv("data get block 233 -28 244", show=False)[-500:])
