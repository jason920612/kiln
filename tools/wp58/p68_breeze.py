"""wp68 #4: wind charges (thrown at the feet: the lift), the mace (smash from a fall, density, breach, wind burst), the breeze (its charge, deflecting it)
and the bogged (shears), Kiln (TARGET=K) vs vanilla (TARGET=V). One actor; par.sh."""
from p68_small import *
import sys

scenario("breeze")


def lines_all():
    return json.loads(a.cmd("spylog clear"))


def health_of(sel):
    out = sv(f"data get entity {sel} Health", show=False, wait=0.5)
    m = re.findall(r"(-?[\d.]+)f\]?\s*$", out.strip())
    return float(m[0]) if m else out[-40:]


def wind():
    reset()
    sv("fill 10 200 10 30 240 30 minecraft:air", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.0)
    give(ME, "minecraft:wind_charge", 6)
    a.select("minecraft:wind_charge")
    out = []
    for k in range(3):
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        time.sleep(1.2)
        a.look(0, 90)  # straight down
        time.sleep(0.4)
        rec_start(a)
        a.mouse("right", "tap")
        ymax = 200.0
        t0 = time.time()
        while time.time() - t0 < 2.5:
            time.sleep(0.05)
            ymax = max(ymax, a.state(0)["y"])
        ls = lines_all()
        out.append((round(ymax, 2), dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:60] for l in ls if "sound" in l and "step" not in l)), [l[:60] for l in ls if "explosion" in l.lower() or "level_event" in l][:3]))
        time.sleep(1.0)
    F("wind charge thrown straight down: peak height, sounds, events:", out)
    # thrown at a zombie: knockback
    sv("kill @e[type=zombie]", show=False)
    sv("summon minecraft:zombie 20.5 200 25.5 {NoAI:1b,Health:100f,Silent:1b,Attributes:[{id:'minecraft:max_health',base:100d}],Tags:[\"t\"]}", show=False)
    time.sleep(1)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1)
    a.lookat(20.5, 200.9, 25.5)
    time.sleep(0.5)
    a.mouse("right", "tap")
    time.sleep(2)
    F("zombie after a charge:", sv("data get entity @e[type=zombie,tag=t,limit=1] Pos", show=False, wait=0.5)[-90:], sv("data get entity @e[type=zombie,tag=t,limit=1] Motion", show=False, wait=0.5)[-90:], "health", health_of("@e[type=zombie,tag=t,limit=1]"))


def mace():
    reset()
    sv("fill 10 200 10 30 260 30 minecraft:air", show=False)
    rows = []
    for name, ench in (("plain", ""), ("density", "density=5"), ("breach", "breach=4"), ("wind", "wind_burst=3")):
        sv("kill @e[type=zombie]", show=False)
        sv("summon minecraft:zombie 20.5 200 20.5 {NoAI:1b,Silent:1b,Health:200f,Attributes:[{id:'minecraft:max_health',base:200d}],Tags:[\"t\"]}", show=False)
        sv(f"clear {ME}", show=False)
        sv(f"give {ME} minecraft:mace[enchantments={{{ench.replace('=', ':')}}}] 1" if ench else f"give {ME} minecraft:mace 1", show=False)
        sv(f"effect clear {ME}", show=False)
        sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
        a.select("minecraft:mace")
        # fall from 10 blocks onto the zombie's head, looking down at it
        sv(f"tp {ME} 20.5 212 20.5", show=False)
        time.sleep(0.3)
        a.lookat(20.5, 201.0, 20.5)
        rec_start(a)
        for _ in range(40):
            a.mouse("left", "tap")
            time.sleep(0.1)
            if a.state(0)["onGround"]:
                break
        time.sleep(0.6)
        ls = lines_all()
        rows.append((name, health_of("@e[type=zombie,tag=t,limit=1]"), dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:60] for l in ls if "sound" in l and "step" not in l)), re.findall(r"damage: \d+", sv(f"data get entity {ME} SelectedItem", show=False))))
        time.sleep(0.5)
    F("mace smash from ~12 blocks (zombie health left of 200, sounds, mace damage):", rows)


def breeze():
    reset()
    sv("fill 10 200 10 30 206 30 minecraft:air", show=False)
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("summon minecraft:breeze 20.5 200 26.5 {Tags:[\"b\"],PersistenceRequired:1b}", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    rec_start(a)
    t0 = time.time()
    charges = 0
    seen = set()
    ents = collections.Counter()
    hits = []
    while time.time() - t0 < 25:
        time.sleep(0.2)
        for e in a.state(30)["entities"]:
            if e["t"] in ("wind_charge", "breeze_wind_charge") and e["id"] not in seen:
                seen.add(e["id"])
                charges += 1
            ents[e["t"]] += 1
    ls = lines_all()
    F("breeze: wind charges seen in 25 s", charges, "types", {k: v for k, v in ents.items() if k not in ("player",)}, "sounds:", dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:60] for l in ls if "sound" in l and "step" not in l).most_common(8)))
    F("breeze nbt:", re.findall(r"Health: [\d.]+f|Brain|memories", sv("data get entity @e[type=breeze,limit=1]", show=False, wait=0.6))[:3])
    # kill it with the sword: loot (breeze rods)
    sv(f"give {ME} minecraft:diamond_sword", show=False)
    a.select("minecraft:diamond_sword")
    sv("effect give @e[type=breeze] minecraft:slowness 10 6 true", show=False)
    got = collections.Counter()
    for _ in range(30):
        kill_nearest(a, "breeze", rng=12, timeout=4)
        if not ents_of(a, "breeze", 30):
            break
    time.sleep(1)
    for e in a.state(12)["entities"]:
        if e["t"] == "item":
            got[e.get("item")] += e.get("n", 1)
    F("breeze loot:", dict(got))
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs false", show=False)


def bogged():
    reset()
    sv("kill @e[type=!player]", show=False)
    sv("summon minecraft:bogged 20.5 200 24.5 {NoAI:1b,Silent:1b,PersistenceRequired:1b,Rotation:[180f,0f]}", show=False)
    give(ME, "minecraft:shears", 1)
    time.sleep(1.2)
    es = ents_of(a, "bogged", 30)
    if not es:
        F("bogged not found")
        return
    eid = es[0]["id"]
    a.select("minecraft:shears")
    rec_start(a)
    ok = interact_ent(a, eid, button="right", tries=8, dy=0.9)
    time.sleep(1.5)
    items = collections.Counter(e.get("item") for e in a.state(10)["entities"] if e["t"] == "item")
    F("bogged sheared: click ok", ok, "items:", dict(items), "sheared flag:", re.findall(r"sheared: \d+b", sv("data get entity @e[type=bogged,limit=1]", show=False, wait=0.5)), "shears damage:", re.findall(r"damage: \d+", sv(f"data get entity {ME} SelectedItem", show=False)), [l[:70] for l in lines_all() if "sound" in l][:3])
    # a second shear: nothing
    ok = interact_ent(a, eid, button="right", tries=3, dy=0.9)
    time.sleep(1)
    F("bogged sheared again:", ok, "items now", dict(collections.Counter(e.get("item") for e in a.state(10)["entities"] if e["t"] == "item")))
    # its arrows are poisoned: shoot it a real one? check the equipment instead
    F("bogged equipment:", re.findall(r"id: \"minecraft:\w+\"", sv("data get entity @e[type=bogged,limit=1] equipment", show=False, wait=0.5))[:3])
    sv("kill @e[type=!player]", show=False)


if __name__ == "__main__":
    prep()
    time.sleep(2)
    for which in sys.argv[1:] or ["wind", "mace", "breeze", "bogged"]:
        try:
            globals()[which]()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
