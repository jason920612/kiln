"""wp84 #3: the mace from a real fall, one actor against each server (par.sh): the player drops from H blocks onto a NoAI zombie (iron armour, 500 health), holding the attack
button and aiming at the zombie's head; the zombie's health taken, the sounds, the player's own motion packets (the smash's 0.01 and the wind burst's push) and where the player
ends up. Plain mace at several heights, then density, breach and wind burst at H=24.
usage: par.sh p84_mace.py [heights comma list] [variants comma list]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
HEIGHTS = [int(x) for x in (sys.argv[1] if len(sys.argv) > 1 else "6,12,24").split(",")]
VARIANTS = (sys.argv[2] if len(sys.argv) > 2 else "plain,density5,breach4,windburst3").split(",")
scenario("mace84")
ENCH = {"plain": "", "density5": '[enchantments={"minecraft:density":5}]', "breach4": '[enchantments={"minecraft:breach":4}]', "windburst3": '[enchantments={"minecraft:wind_burst":3}]'}


def zhp():
    m = re.findall(r"(-?\d+(?:\.\d+)?)f", nbt_of("@e[tag=t,limit=1]", "Health", 0.4))
    return float(m[-1]) if m else None


def drop(variant, h):
    kill_all()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"give {ME} minecraft:mace{ENCH[variant]} 1", show=False)
    sv("summon minecraft:zombie 20.5 200 24.5 {NoAI:1b,PersistenceRequired:1b,Tags:[\"t\"],attributes:[{id:\"minecraft:max_health\",base:500}],Health:500f,"
       "equipment:{head:{id:\"minecraft:iron_helmet\",count:1},chest:{id:\"minecraft:iron_chestplate\",count:1},legs:{id:\"minecraft:iron_leggings\",count:1},feet:{id:\"minecraft:iron_boots\",count:1}}}", show=False)
    time.sleep(1.0)
    a.select("minecraft:mace")
    sv(f"tp {ME} 20.5 {200 + h} 22.5", show=False)
    h0 = zhp()
    a.cmd("spy on"); a.cmd("spylog clear")
    a.mouse("left", "down")
    t0 = time.time()
    peak = pos(a)[1]
    hit_t = None
    while time.time() - t0 < 6 + h / 10:
        s = a.state(0)
        peak = max(peak, s["y"])
        a.lookat(20.5, 201.6, 24.5)
        if hit_t is None and (zhp() or h0) < h0:
            hit_t = time.time() - t0
        time.sleep(0.04)
        if hit_t is not None and time.time() - t0 > hit_t + 2.5:
            break
    a.mouse("left", "up")
    h1 = zhp()
    ls = json.loads(a.cmd("spylog clear"))
    mot = [re.sub(r"(-?\d+\.\d{2})\d*", r"\1", l[12:]) for l in ls if l.startswith("motion_self")]
    snd = sorted({l.split(" ")[1] for l in ls if l.startswith("sound") and "step" not in l and "fall" not in l and "swim" not in l})
    F(variant, "H", h, "zombie health", h0, "->", h1, "taken", None if h0 is None or h1 is None else round(h0 - h1, 2), "player peak y", round(peak, 2), "now", pos(a), "hp", a.state(0)["health"],
      "sounds", snd[:8], "own motion packets", mot[:4])


if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "mob_griefing true", "do_daylight_cycle false", "natural_regeneration false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(10, 10, 30, 34, y=199, clear=45)
for variant in VARIANTS:
    for h in (HEIGHTS if variant == "plain" else [24]):
        try:
            drop(variant, h)
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F("ERROR", variant, h, repr(ex)[:200])
kill_all()
