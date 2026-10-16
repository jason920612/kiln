from p68_small import *
prep(); time.sleep(3)
for typ in ("cow", "pig", "horse", "sheep", "villager"):
    reset()
    m = summon(typ, "{Age:-24000,NoAI:1b}")
    eid = m["id"]
    give(ME, "minecraft:golden_dandelion", 4)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:golden_dandelion")
    time.sleep(1.0)
    ls = lines()
    out = sv(f"data get entity @e[type={typ},limit=1]", show=False)
    inv = sv(f"data get entity {ME} Inventory", show=False)
    F("dandelion", typ, "ok", ok, "server AgeLocked/Age:", re.findall(r"AgeLocked: \dB|AgeLocked: \db|Age: -?\d+", out), "server dandelions:", re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:golden_dandelion"', inv), flt(ls, "dandelion", "pause")[:5])
