"""wp84 #2: spears from the saddle and the lunge enchantment, one actor against each server (par.sh).
 mounted: a tamed saddled horse with a fixed speed; the rider jabs a NoAI pig standing still (left click), then gallops at a pig holding the right button (charge);
 lunge: Lunge I/II/III on foot (jab at nothing): displacement, food, durability, sounds; and charge with lunge.
usage: par.sh p84_spear.py [mounted,lunge]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
PH = (sys.argv[1] if len(sys.argv) > 1 else "mounted,lunge").split(",")
scenario("spear84")


def hp_of(typ="pig"):
    e = [x for x in a.state(60)["entities"] if x["t"] == typ]
    return (e[0]["hp"], round(e[0]["x"], 2), round(e[0]["z"], 2)) if e else None


def lines():
    return json.loads(a.cmd("spylog clear"))


def reset():
    kill_all()
    a.release()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"effect give {ME} minecraft:saturation 99999 0 true", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.2)


def sounds(ls):
    return sorted({l.split(" ")[1] for l in ls if l.startswith("sound") and "step" not in l})[:8]


def dur():
    return re.findall(r"damage: \d+", sv(f"data get entity {ME} SelectedItem", show=False))


def mounted():
    for spear in ("minecraft:iron_spear", "minecraft:netherite_spear"):
        reset()
        sv(f"give {ME} {spear} 1", show=False)
        sv("summon minecraft:horse 20.5 200 16.5 {Tame:1b,PersistenceRequired:1b,Variant:0,Tags:[\"h\"],equipment:{saddle:{id:\"minecraft:saddle\",count:1}},attributes:[{id:\"minecraft:movement_speed\",base:0.3375},{id:\"minecraft:max_health\",base:30}]}", show=False)
        sv(f"data modify entity @e[tag=h,limit=1] Owner set from entity {ME} UUID", show=False)
        sv("summon minecraft:pig 20.5 200 22.5 {NoAI:1b,PersistenceRequired:1b,Rotation:[180f,0f]}", show=False)
        time.sleep(1.5)
        a.select(spear)
        sv(f"ride {ME} mount @e[tag=h,limit=1]", show=False)
        time.sleep(1.0)
        F(spear.split(":")[1], "mounted:", a.state(0)["vehicle"] is not None)
        a.look(0, 10)
        time.sleep(0.5)
        # jab from the saddle (the pig is 4 blocks in front, step the horse close first)
        a.key("w", "down"); time.sleep(0.9); a.key("w", "up"); time.sleep(1.5)
        h0 = hp_of()
        a.lookat(20.5, 200.7, 22.5)
        time.sleep(0.4)
        rec_start(a)
        a.mouse("left", "tap")
        time.sleep(1.2)
        h1 = hp_of()
        ls = lines()
        F(spear.split(":")[1], "mounted jab: pig", h0, "->", h1, "rider z", pos(a)[2], "sounds", sounds(ls), "dur", dur())
        rec_stop(a, "jab_" + spear.split(":")[1]) if False else None
        # charge: back off 20 blocks, then gallop at the second pig holding the right button
        sv("kill @e[type=pig]", show=False)
        sv("summon minecraft:pig 20.5 200 40.5 {NoAI:1b,PersistenceRequired:1b,Rotation:[180f,0f]}", show=False)
        sv(f"tp @e[tag=h,limit=1] 20.5 200 6.5", show=False)
        time.sleep(1.5)
        a.look(0, 5)
        h0 = hp_of()
        rec_start(a)
        a.mouse("right", "down")
        time.sleep(0.2)
        a.key("w", "down"); a.key("ctrl", "down")
        t0 = time.time(); hmin = h0
        while time.time() - t0 < 6.0:
            time.sleep(0.05)
            h = hp_of()
            if h and h0 and h[0] < hmin[0]:
                hmin = h
        a.key("w", "up"); a.key("ctrl", "up"); a.mouse("right", "up")
        time.sleep(1.0)
        ls = lines()
        F(spear.split(":")[1], "mounted charge: pig", h0, "->", hp_of(), "min seen", hmin, "rider z", pos(a)[2], "sounds", sounds(ls), "dur", dur())
        sv(f"ride {ME} dismount", show=False)
        time.sleep(0.5)


def lunge():
    for lvl in (1, 2, 3):
        reset()
        sv(f"give {ME} minecraft:diamond_spear[enchantments={{\"minecraft:lunge\":{lvl}}}] 1", show=False)
        time.sleep(0.5)
        a.select("minecraft:diamond_spear")
        sv(f"effect clear {ME} minecraft:saturation", show=False)
        sv(f"effect give {ME} minecraft:saturation 1 0 true", show=False)
        time.sleep(1.5)
        a.look(0, 0)
        z0 = pos(a)
        f0 = a.state(0)["food"]
        rec_start(a)
        a.mouse("left", "tap")
        time.sleep(1.5)
        z1 = pos(a)
        ls = lines()
        F("lunge", lvl, "jab at air on the ground: moved", [round(z1[i] - z0[i], 2) for i in range(3)], "food", f0, "->", a.state(0)["food"], "sounds", sounds(ls), "dur", dur())
        # second jab right away (cooldown / lunge cost)
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        time.sleep(1.0)
        a.look(0, 0)
        z0 = pos(a)
        a.mouse("left", "tap")
        time.sleep(0.3)
        a.mouse("left", "tap")
        time.sleep(1.5)
        z1 = pos(a)
        F("lunge", lvl, "two jabs 0.3 s apart: moved", [round(z1[i] - z0[i], 2) for i in range(3)], "food", a.state(0)["food"], "dur", dur())
        # in the air: jump then jab
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        time.sleep(1.0)
        a.look(0, 0)
        a.key("space", "tap")
        time.sleep(0.25)
        z0 = pos(a)
        a.mouse("left", "tap")
        time.sleep(1.5)
        z1 = pos(a)
        F("lunge", lvl, "jab in the air: moved", [round(z1[i] - z0[i], 2) for i in range(3)], "food", a.state(0)["food"], "dur", dur())


FN = {"mounted": mounted, "lunge": lunge}
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "mob_griefing true", "do_daylight_cycle false", "natural_regeneration false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 60, y=199, clear=14)
for ph in PH:
    try:
        FN[ph]()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR in", ph, repr(ex)[:300])
kill_all()
