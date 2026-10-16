from p68_trial import *
sv(f"tp {ME} 231.5 -29 231.5", show=False)
t0 = time.time()
items = collections.Counter(); seen = set()
last = None
while time.time() - t0 < 120:
    st = state_of(SPAWNER)
    if st != last:
        F("state", round(time.time() - t0), st, fields_of(spawner_nbt()))
        last = st
    sv("kill @e[type=!player,type=!item,type=!experience_orb]", show=False, wait=0.2)
    for e in a.state(12)["entities"]:
        if e["t"] == "item" and e["id"] not in seen:
            seen.add(e["id"]); items[e.get("item")] += e.get("n", 1)
    a.collect(5, 3)
    if st[0] == "cooldown":
        break
    time.sleep(0.5)
F("items", dict(items), "inventory keys:", {k: v for k, v in a.inv().items() if "key" in k})
