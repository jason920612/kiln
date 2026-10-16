"""wp68 #3: spears (jab with the left button, charge by holding the right button while running), Kiln (TARGET=K) vs vanilla (TARGET=V).
One actor; run with par.sh to drive both servers at once."""
from p68_small import *
import sys

scenario("spear")


def hp_of(eid):
    e = [x for x in a.state(40)["entities"] if x["id"] == eid]
    return (e[0]["hp"], round(e[0]["x"], 2), round(e[0]["z"], 2)) if e else None


def lines_all():
    return json.loads(a.cmd("spylog clear"))


def dummy(typ="pig", x=20.5, z=26.5, extra=",NoAI:1b,PersistenceRequired:1b"):
    clear_area()
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    a.release()
    time.sleep(0.8)
    sv(f"summon minecraft:{typ} {x} 200 {z} {{Rotation:[180f,0f]{extra}}}", show=False)
    time.sleep(1.2)
    es = ents_of(a, typ, 30)
    return es[0]["id"] if es else None


def durability(slot_item):
    return sv(f"data get entity {ME} SelectedItem", show=False)


def jab(spear):
    eid = dummy()
    give(ME, spear, 1)
    time.sleep(0.4)
    a.select(spear)
    a.look(0, 5)
    # walk up to 2.5 blocks from the pig
    a.key("w", "down")
    time.sleep(0.35)
    a.key("w", "up")
    time.sleep(0.5)
    st = pos(a)
    a.lookat(20.5, 200.8, 26.5)
    time.sleep(0.5)
    h0 = hp_of(eid)
    rec_start(a)
    a.mouse("left", "tap")
    time.sleep(1.2)
    h1 = hp_of(eid)
    ls = lines_all()
    F("jab", spear.split(":")[1], "from z", st[2], "pig", h0, "->", h1, "sounds:", [l[:80] for l in ls if "sound" in l and "step" not in l][:5], "durability:", re.findall(r"damage: \d+", durability(spear)))


def charge(spear, secs=4.5, sprint=True):
    eid = dummy(z=32.5)
    give(ME, spear, 1)
    time.sleep(0.4)
    a.select(spear)
    sv(f"tp {ME} 20.5 200 8.5", show=False)
    time.sleep(1.0)
    a.look(0, 15)
    h0 = hp_of(eid)
    rec_start(a)
    a.mouse("right", "down")
    time.sleep(0.15)
    a.key("w", "down")
    if sprint:
        a.key("ctrl", "down")
    t0 = time.time()
    hmin = h0
    while time.time() - t0 < secs:
        time.sleep(0.05)
        h = hp_of(eid)
        if h and h0 and h[0] < hmin[0]:
            hmin = h
    a.key("w", "up")
    if sprint:
        a.key("ctrl", "up")
    a.mouse("right", "up")
    time.sleep(0.8)
    h1 = hp_of(eid)
    ls = lines_all()
    F("charge", spear.split(":")[1], "sprint" if sprint else "walk", "pig", h0, "->", h1, "player z", pos(a)[2], "sounds:", [l[:80] for l in ls if "sound" in l and "step" not in l][:6], "durability:", re.findall(r"damage: \d+", durability(spear)))


def enchanted():
    # lunge: a charge with Lunge III launches the player forward
    eid = dummy(z=34.5)
    sv(f"give {ME} minecraft:iron_spear[enchantments={{\"minecraft:lunge\":3}}] 1", show=False)
    time.sleep(0.5)
    a.select("minecraft:iron_spear")
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.0)
    a.look(0, 0)
    z0 = pos(a)[2]
    rec_start(a)
    a.mouse("left", "tap")
    time.sleep(1.5)
    F("lunge III jab: moved z by", round(pos(a)[2] - z0, 2), "feet", pos(a), "food", a.state(0)["food"], [l[:80] for l in lines_all() if "sound" in l][:4])


if __name__ == "__main__":
    prep()
    time.sleep(3)
    sv(f"effect give {ME} minecraft:saturation 99999 0 true", show=False)
    for fn in sys.argv[1:] or ["all"]:
        try:
            if "nojab" not in sys.argv:
                for sp in ("minecraft:wooden_spear", "minecraft:stone_spear", "minecraft:iron_spear", "minecraft:diamond_spear", "minecraft:netherite_spear"):
                    jab(sp)
            for sp in ("minecraft:wooden_spear", "minecraft:iron_spear", "minecraft:netherite_spear"):
                charge(sp, sprint=True)
                charge(sp, sprint=False)
            enchanted()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F("ERROR", repr(ex)[:300])
