"""wp72: like p72_one, but the commands of the 2nd list run after a settle (30 ticks): usage p72_one2.py TICKS 'setup cmd'... -- 'trigger cmd'..."""
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
    w(PLAY / "console", cmd); w(p61.VDIR / "console", cmd)
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
    i = sys.argv.index("--")
    setup, trig = sys.argv[2:i], sys.argv[i + 1:]
    for g in ("random_tick_speed 0", "spawn_mobs false", "mob_griefing false"):
        both_("gamerule " + g)
    both_("tick freeze")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 30"); time.sleep(1.5)
    for c in setup:
        both_(c)
    time.sleep(0.3)
    both_("tick step 30"); time.sleep(1.5)
    for c in trig:
        both_(c)
    time.sleep(0.4)
    prev = None
    import re
    def gt(path):
        txt = open(path, errors='replace').read()
        m = re.findall(r"gametime\[(\d+)\]|game time is (\d+)", txt)
        return [a or b for a, b in m][-1] if m else None
    for t in range(0, ticks + 1):
        if t:
            both_("tick step 1"); time.sleep(0.35)
        both_("time query gametime"); time.sleep(0.2)
        print("   gametime K", gt(PLAY / "server.log"), "V", gt(p61.VDIR / "server.log"))
        kd, vd = stable(ka), stable(va)
        if prev is None or (kd, vd) != prev:
            print(f"t={t} {'same' if kd == vd else 'DIFF'}")
            if kd == vd:
                for s in sorted(kd): print("   ", s)
            else:
                for s in sorted(kd - vd): print("   K only:", s)
                for s in sorted(vd - kd): print("   V only:", s)
                for s in sorted(kd & vd): print("   both:  ", s)
        prev = (kd, vd)
main()
