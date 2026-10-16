from p68_small import *
sv("kill @e[tag=nai]", show=False)
sv(f"tp {ME} 20.5 200 30.5", show=False)
for k in range(4):
    sv("kill @e[type=item]", show=False)
    sv("kill @e[tag=nai]", show=False)
    sv(f"clear {ME}", show=False)
    sv("summon minecraft:cow 22.5 200 30.5 {Tags:[\"nai\"],NoAI:1b}", show=False)
    give(ME, "minecraft:lead", 3)
    time.sleep(1)
    cow = ents_of(a, "cow", 6)
    ok = use_on_entity(a, cow[0]["id"], "minecraft:lead", dy=0.5)
    n1 = a.inv().get("minecraft:lead")
    ok2 = use_on_entity(a, cow[0]["id"], "minecraft:lead", dy=0.5)
    ts = []
    for _ in range(6):
        time.sleep(0.25)
        ts.append([(round(e["x"], 2), round(e["y"], 2), round(e["z"], 2)) for e in a.state(10)["entities"] if e["t"] == "item"])
    print(k, ok, n1, ok2, "leads:", a.inv().get("minecraft:lead"), ts[0], ts[-1])
sv("kill @e[tag=nai]", show=False)
