"""wp72: run one hand-written circuit on both servers and print the box dump of both after every tick.
usage: p72_one.py TICKS 'setblock ...' 'setblock ...' ...   (coordinates inside x 10..18, y 200..204, z 10..18; floor at y 199)"""
import sys, time, json
import p61
from p61 import *
from ctl import C, PLAY

p61.VDIR = HOME / "wp72" / "vanilla"
X0, Y0, Z0, W, H, D = 10, 200, 10, 9, 5, 9
ka, va = C(25999), C(26000)


def w(path, cmd):
    with open(path, "w") as f:
        f.write(cmd + "\n")


def both_(cmd):
    w(PLAY / "console", cmd)
    w(p61.VDIR / "console", cmd)


def dump(c):
    return set(json.loads(c.cmd(f"boxdump {X0} {Y0} {Z0} {X0 + W - 1} {Y0 + H} {Z0 + D - 1}")))


def stable(c):
    prev = dump(c)
    for _ in range(12):
        time.sleep(0.12)
        cur = dump(c)
        if cur == prev:
            return cur
        prev = cur
    return prev


def main():
    ticks = int(sys.argv[1])
    cmds = sys.argv[2:]
    for g in ("random_tick_speed 0", "spawn_mobs false", "mob_griefing false"):
        both_("gamerule " + g)
    both_("tick freeze")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 30")
    time.sleep(2.2)
    for c in cmds:
        both_(c)
    time.sleep(0.4)
    prev = None
    for t in range(0, ticks + 1):
        if t:
            both_("tick step 1")
            time.sleep(0.35)
        kd, vd = stable(ka), stable(va)
        tag = "same" if kd == vd else "DIFF"
        if prev is None or (kd, vd) != prev:
            print(f"t={t} {tag}")
            if kd == vd:
                for s in sorted(kd):
                    print("   ", s)
            else:
                for s in sorted(kd - vd):
                    print("   K only:", s)
                for s in sorted(vd - kd):
                    print("   V only:", s)
                for s in sorted(kd & vd):
                    print("   both:  ", s)
        prev = (kd, vd)


main()
