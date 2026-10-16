"""wp72: natural spawn survey. The actor stands (creative, still) at the same place on Kiln and on vanilla (par.sh runs this on both at once),
`tick sprint` runs ROUNDS x ROUND_TICKS ticks; after every round every mob type that spawns naturally is killed and counted
(`execute store result score <type> n run kill @e[type=<type>]`, read back with `scoreboard players list`).
usage: p72_survey.py SITE...     (SITE = a key of SITES)"""
import sys, time, re, json
from p72 import *

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
ROUNDS = int(os.environ.get("ROUNDS", "4"))
ROUND_TICKS = int(os.environ.get("ROUND_TICKS", "1500"))

TYPES = """zombie zombie_villager husk drowned skeleton stray bogged parched creeper spider cave_spider enderman endermite witch slime phantom
silverfish pillager guardian elder_guardian zombified_piglin piglin piglin_brute hoglin zoglin ghast blaze magma_cube wither_skeleton strider shulker
cod salmon pufferfish tropical_fish squid glow_squid dolphin turtle axolotl bat camel_husk creaking breeze vex""".split()

# key: (dimension, biome to locate or fixed x z, night?)
SITES = {
    "plains": ("overworld", "minecraft:plains", True),
    "dark_forest": ("overworld", "minecraft:dark_forest", True),
    "swamp": ("overworld", "minecraft:swamp", True),
    "desert": ("overworld", "minecraft:desert", True),
    "snowy_taiga": ("overworld", "minecraft:snowy_taiga", True),
    "jungle": ("overworld", "minecraft:jungle", True),
    "badlands": ("overworld", "minecraft:badlands", True),
    "mushroom": ("overworld", "minecraft:mushroom_fields", True),
    "deep_ocean": ("overworld", "minecraft:deep_ocean", True),
    "warm_ocean": ("overworld", "minecraft:warm_ocean", True),
    "frozen_ocean": ("overworld", "minecraft:frozen_ocean", True),
    "river": ("overworld", "minecraft:river", True),
    "soul_sand_valley": ("the_nether", "minecraft:soul_sand_valley", False),
    "basalt_deltas": ("the_nether", "minecraft:basalt_deltas", False),
    "nether_wastes": ("the_nether", "minecraft:nether_wastes", False),
    "end_main": ("the_end", (0, 0), False),
    "end_highlands": ("the_end", "minecraft:end_highlands", False),
}


def log_since(n):
    path = (PLAY / "server.log") if T == "K" else (p61.VDIR / "server.log")
    with open(path, "rb") as f:
        f.seek(n)
        return ANSI.sub("", f.read().decode(errors="replace"))


def logpos():
    path = (PLAY / "server.log") if T == "K" else (p61.VDIR / "server.log")
    return path.stat().st_size


def locate_biome(biome, dim):
    for dist in (0,):
        n = logpos()
        sv(f"execute in minecraft:{dim} positioned 0 64 0 run locate biome {biome}", wait=6, show=False)
        txt = log_since(n)
        m = re.search(r"\[(-?\d+), (-?\d+|~), (-?\d+)\]", txt)
        if m:
            return int(m.group(1)), int(m.group(3))
    return None


def counts():
    """Kill every natural mob type in the world, return {type: n}. Kiln logs translation keys (commands.kill.success.multiple[N] / .single[name]), vanilla English."""
    out = {}
    for t in TYPES:
        n = logpos()
        sv(f"kill @e[type=minecraft:{t}]", wait=0.12, show=False)
        txt = log_since(n)
        m = re.search(r"success\.multiple\[(\d+)\]|Killed (\d+) entities", txt)
        if m:
            out[t] = int(m.group(1) or m.group(2))
        elif "success.single" in txt or re.search(r"Killed [A-Z]", txt):
            out[t] = 1
    return out


def prep():
    if T == "V":
        sv("op KilnTest", show=False)
    alive_auto(a)
    for g in ("spawn_mobs true", "spawn_monsters true", "do_daylight_cycle false", "do_weather_cycle false", "spawn_phantoms false", "mob_griefing false", "random_tick_speed 3"):
        sv("gamerule " + g, show=False)
    sv("difficulty normal", show=False)
    sv("weather clear", show=False)
    sv(f"gamemode creative {ME}", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)


def go_site(key):
    dim, where, night = SITES[key]
    if isinstance(where, str):
        xz = locate_biome(where, dim)
        if xz is None:
            F(key, "biome not found")
            return None
    else:
        xz = where
    sv(f"time set {'18000' if night else '6000'}", show=False)
    y = 330 if dim == "overworld" else (100 if dim == "the_nether" else 100)
    sv(f"execute in minecraft:{dim} run tp {ME} {xz[0] + 0.5} {y} {xz[1] + 0.5}", show=False)
    sv(f"effect give {ME} minecraft:slow_falling 40 0 true", show=False)
    return xz


def survey(key):
    scenario(f"survey_{key}")
    xz = go_site(key)
    if xz is None:
        return
    time.sleep(45)  # terrain around the landing spot
    st = a.state(0)
    F(key, "at", xz, "player", [round(st["x"]), round(st["y"]), round(st["z"])])
    F(key, "block under", a.block(math.floor(st["x"]), math.floor(st["y"]) - 1, math.floor(st["z"])))
    counts()  # discard what chunk generation made
    total = {}
    for r in range(ROUNDS):
        sprint(ROUND_TICKS)
        c = counts()
        for t, v in c.items():
            total[t] = total.get(t, 0) + v
        F(key, "round", r, {t: v for t, v in c.items() if v})
    F(key, "TOTAL", json.dumps({t: v for t, v in sorted(total.items()) if v}))
    return total


def sprint(n):
    pos = logpos()
    sv(f"tick sprint {n}", wait=0.5, show=False)
    t0 = time.time()
    while time.time() - t0 < 1500:
        txt = log_since(pos)
        if "sprint.report" in txt or "Sprint completed" in txt:
            return
        time.sleep(1)


if __name__ == "__main__":
    prep()
    for key in sys.argv[1:]:
        survey(key)
