"""wp68 #5: sniffer: a sniffer egg on moss hatches (random ticks, cracking stages), the baby is fed torchflower seeds and grows up; an adult
digs seeds out of dirt/moss when given time; Kiln vs vanilla."""
from p68_small import *
import sys

scenario("sniffer")


def run():
    prep()
    time.sleep(2)
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 6.5 200 6.5", show=False)
    time.sleep(1.0)
    sv("fill 20 199 20 22 199 22 minecraft:moss_block", show=False)
    sv("setblock 21 200 21 minecraft:sniffer_egg[hatch=0]", show=False)
    time.sleep(0.8)
    F("egg:", blk(a, 21, 200, 21))
    sv("gamerule random_tick_speed 3000", show=False)
    rec_start(a)
    stages = []
    t0 = time.time()
    for i in range(60):
        sv("tick sprint 1200", show=False, wait=0.4)
        for _ in range(60):
            time.sleep(0.5)
            if "sprinting" not in sv("tick query", show=False, wait=0.3).lower():
                break
        time.sleep(0.5)
        b = blk(a, 21, 200, 21)
        if not stages or stages[-1][1] != b:
            stages.append((i, b))
        if "sniffer_egg" not in b:
            break
    sv("gamerule random_tick_speed 3", show=False)
    F("egg stages (sprint step, block):", stages)
    ls = lines()
    F("hatch sounds:", dict(collections.Counter(re.sub(r"vol=\S+", "", l)[:70] for l in ls if "sound" in l and "sniffer_egg" in l)))
    time.sleep(1.0)
    sv(f"tp {ME} 21.5 200 19.5", show=False)
    time.sleep(1.5)
    es = ents_of(a, "sniffer", 30)
    F("sniffers after hatching:", [(e["id"], e.get("baby"), e["bbh"], round(e["x"], 1), round(e["z"], 1)) for e in es], "server:", re.findall(r"Age: -?\d+", sv("data get entity @e[type=sniffer,limit=1]", show=False)))
    if es:
        eid = es[0]["id"]
        sv("data merge entity @e[type=sniffer,limit=1] {NoAI:1b}", show=False)
        give(ME, "minecraft:torchflower_seeds", 8)
        for i in range(2):
            ok = use_on_entity(a, eid, "minecraft:torchflower_seeds", dy=0.4)
            time.sleep(0.8)
            F("fed seeds", i + 1, ok, "server Age", re.findall(r"Age: -?\d+", sv("data get entity @e[type=sniffer,limit=1]", show=False)), "seeds left on the server", re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:torchflower_seeds"', sv(f"data get entity {ME} Inventory", show=False)))
        sv("data merge entity @e[type=sniffer,limit=1] {Age:-30}", show=False)
        time.sleep(4)
        F("grown:", [(e.get("baby"), e["bbh"]) for e in ents_of(a, "sniffer", 30)])


if __name__ == "__main__":
    try:
        run()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
