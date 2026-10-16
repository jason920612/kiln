"""wp68 #5: archaeology with a real brush (hold the use key on suspicious sand and gravel), the armadillo's scute with the brush,
a decorated pot made from sherds and broken, Kiln vs vanilla. One actor; par.sh."""
from p68_small import *
import sys

scenario("brush")


def brush_block(kind, seed, table):
    reset()
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.0)
    sv(f"setblock 20 200 23 minecraft:{kind}{{LootTable:\"{table}\",LootTableSeed:{seed}L}}", show=False)
    time.sleep(0.5)
    give(ME, "minecraft:brush", 1)
    a.select("minecraft:brush")
    a.lookat(20.5, 200.9, 23.5)
    time.sleep(0.8)
    F(kind, "aim:", a.state(0).get("hit"))
    rec_start(a)
    t0 = time.time()
    a.mouse("right", "down")
    stages = []
    seen = set()
    items = collections.Counter()
    while time.time() - t0 < 12:
        time.sleep(0.1)
        b = blk(a, 20, 200, 23)
        if not stages or stages[-1][1] != b:
            stages.append((round(time.time() - t0, 1), b))
        for e in a.state(8)["entities"]:
            if e["t"] == "item" and e["id"] not in seen:
                seen.add(e["id"])
                items[e.get("item")] += e.get("n", 1)
        if "suspicious" not in b and items:
            break
    a.mouse("right", "up")
    ls = lines()
    F(kind, "brush stages (s, block):", stages)
    F(kind, "items:", dict(items), "brush damage:", re.findall(r"damage: \d+", sv(f"data get entity {ME} SelectedItem", show=False)), "sounds/particles:", dict(collections.Counter(re.sub(r"vol=\S+", "", l)[:60] for l in ls if "sound" in l or "particle" in l or "level_event" in l)))


def armadillo():
    reset()
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.0)
    sv("summon minecraft:armadillo 20.5 200 23.5 {NoAI:1b}", show=False)
    time.sleep(1.2)
    es = ents_of(a, "armadillo", 20)
    eid = es[0]["id"]
    give(ME, "minecraft:brush", 1)
    a.select("minecraft:brush")
    rec_start(a)
    ok = interact_ent(a, eid, button="right", tries=8, dy=0.3)
    time.sleep(1.5)
    items = [(e.get("item"), e.get("n")) for e in a.state(8)["entities"] if e["t"] == "item"]
    a.collect(5, 3)
    F("armadillo brushed ok", ok, "items on the floor", items, "scutes in the inventory:", a.inv().get("minecraft:armadillo_scute", 0), "brush damage:", re.findall(r"damage: \d+", sv(f"data get entity {ME} SelectedItem", show=False)), [l[:70] for l in lines() if "sound" in l][:5])
    F("armadillo NBT:", re.findall(r"state: \"\w+\"|scute_time: \d+", sv("data get entity @e[type=armadillo,limit=1]", show=False)))


def pot():
    reset()
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.0)
    sv("clear KilnTest", show=False)
    for s in ("minecraft:angler_pottery_sherd", "minecraft:archer_pottery_sherd", "minecraft:arms_up_pottery_sherd", "minecraft:blade_pottery_sherd"):
        give(ME, s, 1)
    give(ME, "minecraft:brick", 4)
    give(ME, "minecraft:crafting_table", 1)
    sv("setblock 20 200 23 minecraft:crafting_table", show=False)
    time.sleep(0.5)
    a.lookat(20.5, 200.5, 23.5)
    time.sleep(0.5)
    a.mouse("right", "tap")
    time.sleep(1.2)
    F("crafting screen:", a.screen())
    if a.screen():
        st = a.state(0)
        slots = {s["id"].split(":")[1]: s["i"] for s in st["slots"] if s["id"]}
        # pattern: sherd top, sherds left+right, brick bottom (a "plus" shape)
        place = {2: "angler_pottery_sherd", 4: "archer_pottery_sherd", 6: "arms_up_pottery_sherd", 8: "brick"}
        for gi, name in place.items():
            a.guiclick(slots[name], "left")
            a.guiclick(gi, "right")
            time.sleep(0.2)
        time.sleep(0.6)
        st = a.state(0)
        F("crafting result slot:", st["slots"][0], "carried", st.get("carried"))
        a.guiclick(0, "left")
        time.sleep(0.5)
        F("carried after taking the result:", a.state(0).get("carried"))
        a.guiclick(slots["crafting_table"] if "crafting_table" in slots else 10, "left")
        a.key("esc", "tap")
    time.sleep(0.6)
    inv = a.inv()
    F("decorated pot in inventory:", inv.get("minecraft:decorated_pot"), sv(f"data get entity {ME} Inventory", show=False)[-400:])


if __name__ == "__main__":
    prep()
    time.sleep(2)
    for which in sys.argv[1:] or ["sand", "gravel", "armadillo", "pot"]:
        try:
            if which == "sand":
                brush_block("suspicious_sand", 11, "minecraft:archaeology/desert_pyramid")
            elif which == "gravel":
                brush_block("suspicious_gravel", 12, "minecraft:archaeology/trail_ruins_common")
            else:
                globals()[which]()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
