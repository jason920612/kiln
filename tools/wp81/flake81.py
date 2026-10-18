"""wp81 item 5: the `data get entity Diff0 Health` 19-against-18 flake of command_diff, explained. `damage` right after an earlier hurt is only the difference
(`LivingEntity.hurtServer`: while `invulnerableTime > 10` a hit of `amount <= lastHurt` is ignored and a bigger one costs `amount - lastHurt`; the window is
20 ticks, the second half of it counts), so whether `damage W0 2` costs 2 or 1 depends on how many ticks ago the player was last hurt: the section after
`ride ... dismount` / `gamemode survival` of command_diff.py ran just after the player landed. Here a first hit of 1 is followed by `damage W0 2` D ms later,
on Kiln (K) and vanilla (V) at their own pace; the Health read per delay D has to agree (the boundary is at 500 ms, 10 ticks).
usage: flake81.py [REPEATS]   needs both servers up (sv.sh), the player W0 in (bot.sh)"""
import collections, re, sys, time
from lk import *

N = int(sys.argv[1]) if len(sys.argv) > 1 else 10
DELAYS_MS = [0, 100, 200, 300, 400, 450, 500, 550, 600, 800, 1200]
ME = "W0"
hist = {t: collections.defaultdict(collections.Counter) for t in "KV"}


def health(t):
    out = ask(t, f"data get entity {ME} Health", wait=0.3)
    m = re.findall(r"(-?\d+(?:\.\d+)?)f", out)
    return m[-1] if m else out.strip()[-40:]


for t in "KV":
    w(t, "gamerule spawn_mobs false")
    w(t, "difficulty normal")
    w(t, f"gamemode survival {ME}")
    time.sleep(1.0)
for rep in range(N):
    for d in DELAYS_MS:
        for t in "KV":
            w(t, "gamerule natural_health_regeneration true")
            w(t, f"effect give {ME} minecraft:instant_health 1 10 true")
            w(t, f"effect give {ME} minecraft:saturation 1 10 true")
        time.sleep(1.3)
        for t in "KV":
            w(t, "gamerule natural_health_regeneration false")
        time.sleep(0.2)
        for t in "KV":
            w(t, f"damage {ME} 1")
        time.sleep(d / 1000.0)
        for t in "KV":
            w(t, f"damage {ME} 2")
        for t in "KV":
            hist[t][d][health(t)] += 1
print("delay ms   Kiln                         vanilla")
for d in DELAYS_MS:
    print("%8d   %-28s %s" % (d, dict(hist["K"][d]), dict(hist["V"][d])))
