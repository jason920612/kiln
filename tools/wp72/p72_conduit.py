"""wp72: conduit, Kiln against vanilla (frozen, stepped tick by tick on both, game time aligned mod 40).
A 17x17x17 glass box filled with water, a conduit with the 42-block prismarine frame in its middle, a mockable zombie and the player in the
water. Prints the tick (after the build) at which the player gets Conduit Power, the zombie's health after every tick, the remaining
duration of the power and whether the conduit's block entity names a target. usage: p72_conduit.py [full|short]"""
import sys, time, re, json
import p61
from p61 import *
from ctl import C, PLAY
from lock import respawn_all

p61.VDIR = HOME / "wp72" / "vanilla"
X, Y, Z = 30, 210, 30
MODE = sys.argv[1] if len(sys.argv) > 1 else "full"
ka, va = C(25999), C(26000)
LOGS = {"K": PLAY / "server.log", "V": p61.VDIR / "server.log"}
CONS = {"K": PLAY / "console", "V": p61.VDIR / "console"}


def w(t, cmd):
    with open(CONS[t], "w") as f:
        f.write(cmd + "\n")


def both_(cmd):
    w("K", cmd)
    w("V", cmd)


def logpos(t):
    return LOGS[t].stat().st_size


def since(t, n):
    with open(LOGS[t], "rb") as f:
        f.seek(n)
        return re.sub(r"\x1b\[[0-9;]*m", "", f.read().decode(errors="replace"))


def ask(t, cmd, wait=0.35):
    n = logpos(t)
    w(t, cmd)
    time.sleep(wait)
    return since(t, n)


def gametime(t):
    out = ask(t, "time query gametime")
    m = re.search(r"gametime\[(\d+)\]|game time is (\d+)", out)
    return int(m.group(1) or m.group(2))


def health(t):
    out = ask(t, "data get entity @e[type=minecraft:zombie,limit=1] Health")
    m = re.search(r"(-?\d+\.\d+)f", out)
    return float(m.group(1)) if m else None


def target(t):
    out = ask(t, f"data get block {X} {Y} {Z} Target")
    return "found" if re.search(r"\[I;", out) else "none"


def power(c):
    st = c.state(0)
    for e in st.get("effects", []):
        if "conduit_power" in json.dumps(e):
            return {"duration": e.get("ticks")}
    return None


def step(n=1):
    both_(f"tick step {n}")
    time.sleep(0.4 + 0.06 * n)


def main():
    respawn_all()
    w("V", "op KilnTest")
    for g in ("random_tick_speed 0", "spawn_mobs false", "mob_griefing false", "do_weather_cycle false"):
        both_("gamerule " + g)
    both_("forceload add 0 0 60 60")
    time.sleep(2)
    both_("tick freeze")
    both_("gamemode survival KilnTest")
    both_("effect clear KilnTest")
    both_(f"tp KilnTest {X + 0.5} {Y + 12} {Z + 0.5}")
    both_("effect give KilnTest minecraft:slow_falling 999 0 true")
    both_("kill @e[type=!player]")
    # clear the area and build
    both_(f"fill {X - 9} {Y - 9} {Z - 9} {X + 9} {Y + 9} {Z + 9} minecraft:air")
    step(30)
    time.sleep(2.5)
    for t in "KV":
        gt = gametime(t)
        print(t, "game time", gt, "mod 40 =", gt % 40)
    gts = {t: gametime(t) for t in "KV"}
    for t in "KV":
        need = (38 - gts[t]) % 40  # steps so that the next tick is game time = 39 mod 40
        w(t, f"tick step {need}") if need else None
    time.sleep(0.5 + 0.06 * 40)
    gts = {t: gametime(t) for t in "KV"}
    print("aligned:", gts, {t: g % 40 for t, g in gts.items()})
    # build
    both_(f"fill {X - 8} {Y - 8} {Z - 8} {X + 8} {Y + 8} {Z + 8} minecraft:glass outline")
    both_(f"fill {X - 7} {Y - 7} {Z - 7} {X + 7} {Y + 7} {Z + 7} minecraft:water")
    both_(f"fill {X - 2} {Y - 2} {Z - 2} {X + 2} {Y + 2} {Z + 2} minecraft:prismarine outline")
    both_(f"setblock {X} {Y} {Z} minecraft:conduit[waterlogged=true]")
    if MODE == "short":
        both_(f"setblock {X} {Y + 2} {Z} minecraft:stone")
        both_(f"setblock {X + 2} {Y} {Z} minecraft:stone")
    both_(f"summon minecraft:zombie {X + 5.5} {Y - 1.0} {Z + 0.5} {{NoAI:1b,PersistenceRequired:1b}}")
    both_(f"tp KilnTest {X + 0.5} {Y - 5.0} {Z + 5.5}")
    both_("effect clear KilnTest")
    both_("effect give KilnTest minecraft:resistance 999 4 true")
    time.sleep(1.5)
    rows = {"K": [], "V": []}
    for k in range(0, 125):
        if k:
            step(1)
        for t, c in (("K", ka), ("V", va)):
            if k in (1, 2, 3, 4, 5, 40, 41, 42, 43, 44, 45, 80, 81, 82, 83, 84, 85, 120, 121, 122, 123):
                rows[t].append((k, health(t), power(c) and power(c).get("duration", "?"), target(t) if k in (1, 2, 41, 42, 81, 82, 121) else "-"))
    for t in "KV":
        print(t)
        for r in rows[t]:
            print("  ", r)
    print("SAME" if [r[:2] for r in rows["K"]] == [r[:2] for r in rows["V"]] else "DIFFERENT (health)")
    both_("tick unfreeze")


main()
