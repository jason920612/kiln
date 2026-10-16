"""wp68 #1: copper golem (built from a copper block and a carved pumpkin, sorting into chests, wax/axe/shears, oxidation, statue),
copper chests, Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p68_small import *

scenario("copper")

def chest_items(x, y, z):
    out = sv(f"data get block {x} {y} {z} Items", show=False, wait=0.5)
    items = re.findall(r'count: (\d+), Slot: (\d+)b, id: "minecraft:([a-z_]+)"', out)
    c = {}
    for n, slot, name in items:
        c[name] = c.get(name, 0) + int(n)
    # (older layout: id before count)
    for name, n in re.findall(r'id: "minecraft:([a-z_]+)", count: (\d+)', out):
        c[name] = c.get(name, 0) + int(n)
    return c

def build():
    reset()
    sv("fill 24 200 24 26 202 26 minecraft:air", show=False)
    time.sleep(0.5)
    sv(f"tp {ME} 25.5 200 22.5", show=False)
    time.sleep(1.0)
    a.place("minecraft:copper_block", 25, 199, 25, "up") if False else None
    give(ME, "minecraft:copper_block", 2); give(ME, "minecraft:carved_pumpkin", 2)
    a.select("minecraft:copper_block")
    st = pos(a)
    a.lookat(25.5, 199.99, 25.5)
    time.sleep(0.4)
    F("aim at floor", a.state(0).get("hit"))
    a.mouse("right", "tap"); time.sleep(0.6)
    F("copper block placed:", blk(a, 25, 200, 25))
    a.select("minecraft:carved_pumpkin")
    a.lookat(25.5, 200.99, 25.5)
    time.sleep(0.4)
    F("aim at copper block top", a.state(0).get("hit"))
    rec_start(a)
    a.mouse("right", "tap"); time.sleep(1.5)
    gs = ents_of(a, "copper_golem")
    F("golem built:", [(g["id"], round(g["x"], 1), round(g["z"], 1)) for g in gs], "blocks now:", blk(a, 25, 200, 25), blk(a, 25, 201, 25))
    ls = lines()
    F("golem spawn lines:", flt(ls, "sound", "particle", "level_event", "AddEntity")[:8])
    F("achievements/advancements:", sv("advancement test KilnTest only minecraft:adventure/summon_iron_golem", show=False)[-80:])
    return gs[0]["id"] if gs else None

def sort():
    reset()
    sv("fill 8 200 18 20 201 28 minecraft:air", show=False)
    sv("setblock 12 200 20 minecraft:copper_chest[facing=south]", show=False)
    sv("setblock 16 200 20 minecraft:chest[facing=south]", show=False)
    sv("setblock 12 200 25 minecraft:chest[facing=north]", show=False)
    time.sleep(0.6)
    for slot, (item, n) in enumerate((("diamond", 3), ("iron_ingot", 3), ("gold_ingot", 3), ("emerald", 2))):
        sv(f"item replace block 12 200 20 container.{slot} with minecraft:{item} {n}", show=False)
    sv("item replace block 16 200 20 container.0 with minecraft:diamond 1", show=False)
    sv("item replace block 12 200 25 container.0 with minecraft:iron_ingot 1", show=False)
    F("start: source", chest_items(12, 200, 20), "T1", chest_items(16, 200, 20), "T2", chest_items(12, 200, 25))
    m = summon("copper_golem", "{}", x=14.5, z=22.5)
    rec_start(a)
    t0 = time.time()
    seen_states = []
    while time.time() - t0 < 100:
        time.sleep(2.0)
        es = ents_of(a, "copper_golem")
        if es:
            d = a.cmd(f"entdata {es[0]['id']}")
            if not seen_states or seen_states[-1][1] != d[:60]:
                seen_states.append((round(time.time() - t0), d[:60], round(es[0]["x"], 1), round(es[0]["z"], 1)))
    F("states over 100 s:", seen_states[:30])
    F("end: source", chest_items(12, 200, 20), "T1", chest_items(16, 200, 20), "T2", chest_items(12, 200, 25))
    ls = lines()
    cnt = collections.Counter(re.sub(r"vol=\S+", "", l)[:110] for l in ls if "sound" in l or "block_event" in l or "particle" in l)
    F("sound/block events:", dict(cnt))
    return m

def interact():
    reset()
    m = summon("copper_golem", "{}", x=20.5, z=23.5)
    eid = m["id"]
    rec_start(a)
    for item, label in (("minecraft:honeycomb", "wax"), ("minecraft:iron_axe", "axe-unwax"), ("minecraft:iron_axe", "axe-scrape(unaffected: nothing)"), ("minecraft:shears", "shears")):
        give(ME, item, 1)
        ok = use_on_entity(a, eid, item)
        time.sleep(0.8)
        ls = lines()
        F("golem +", label, "ok", ok, flt(ls, "sound", "level_event", "particle")[:6], "nbt:", re.findall(r"next_weather_age: -?\d+L|weather_state: \"\w+\"", sv("data get entity @e[type=copper_golem,limit=1]", show=False)))
    F("equipment after shears:", a.cmd(f"ent {eid}")[:300])

def weather():
    reset()
    m = summon("copper_golem", "{}", x=20.5, z=23.5)
    eid = m["id"]
    seq = []
    for step in range(5):
        sv("data merge entity @e[type=copper_golem,limit=1] {next_weather_age:1L}", show=False)
        time.sleep(1.5)
        d = a.cmd(f"entdata {eid}") if ents_of(a, "copper_golem") else "gone"
        out = sv("data get entity @e[type=copper_golem,limit=1]", show=False)
        seq.append((re.findall(r'weather_state: "\w+"', out), d[:50]))
        if "gone" == d:
            break
    F("weathering steps:", seq)
    F("statue now:", sv("execute if entity @e[type=copper_golem]", show=False)[-60:], a.find("copper_golem_statue", 20, 5))
    # axe on statue; empty hand on statue
    for (x, y, z) in a.find("copper_golem_statue", 20, 3):
        F("statue block", blk(a, x, y, z))
        a.select("minecraft:iron_axe") if a.inv().get("minecraft:iron_axe") else None
        give(ME, "minecraft:iron_axe", 1); a.select("minecraft:iron_axe")
        ok = a.use_on(x, y, z, face_offset=(0.5, 0.5, 0.5)) if hasattr(a, "use_on") else None
        time.sleep(1.0)
        F("axe on statue:", ok, "golems now:", len(ents_of(a, "copper_golem")), blk(a, x, y, z))
        break

if __name__ == "__main__":
    prep()
    time.sleep(4)
    for fn in sys.argv[1:] or ["build", "sort", "interact", "weather"]:
        try:
            globals()[fn]()
        except Exception as ex:
            import traceback; traceback.print_exc()
            F(fn, "ERROR", repr(ex)[:200])
