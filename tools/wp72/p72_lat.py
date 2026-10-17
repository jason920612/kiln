"""wp72: latency of an observer pulse after setblock at `tick rate 1` (no freeze), Kiln vs vanilla.
The pulse starts on the 2nd tick boundary after the setblock in vanilla (scheduleTick(pos, 2)): 1.0..2.0 s later."""
import sys, time, json, random
import p61
from p61 import *
from ctl import C, PLAY

p61.VDIR = HOME / "wp72" / "vanilla"


def w(path, cmd):
    with open(path, "w") as f:
        f.write(cmd + "\n")


def blk(c, x, y, z):
    return c.cmd(f"block {x} {y} {z}")


def trial(which, c, path, x):
    w(path, f"setblock {x} 200 12 minecraft:air")
    time.sleep(2.5 + random.random())
    w(path, f"setblock {x} 200 12 minecraft:observer[facing=west]")
    t0 = time.time()
    on = None
    while time.time() - t0 < 5:
        if "powered=true" in blk(c, x, 200, 12):
            on = time.time() - t0
            break
        time.sleep(0.03)
    return on


def main():
    for g in ("random_tick_speed 0", "spawn_mobs false"):
        for p in (PLAY, p61.VDIR):
            w(p / "console", "gamerule " + g)
    ka, va = C(25999), C(26000)
    for p in (PLAY, p61.VDIR):
        w(p / "console", "tick unfreeze")
        w(p / "console", "tick rate 1")
    time.sleep(3)
    res = {"K": [], "V": []}
    for i in range(16):
        for name, c, p in (("K", ka, PLAY / "console"), ("V", va, p61.VDIR / "console")):
            res[name].append(trial(name, c, p, 12 + (i % 4)))
    for p in (PLAY, p61.VDIR):
        w(p / "console", "tick rate 20")
    for k, v in res.items():
        print(k, [round(x, 2) if x else None for x in v], "min", round(min(x for x in v if x), 2))


main()
