"""wp77 item 10: the sequence of command_diff that once gave `data get entity Diff0 Health` 19 on one server and 18 on the other, repeated on Kiln (K) and
vanilla (V) with random pauses: gamerule natural_health_regeneration false; damage W0 2; data get entity W0 Health; gamerule ... true.
usage: flake10.py [N]   prints the histogram of the Health read after the damage per server."""
import collections, random, re, sys, time
from lk import *

N = int(sys.argv[1]) if len(sys.argv) > 1 else 200
ME = "W0"
hist = {"K": collections.Counter(), "V": collections.Counter()}
rnd = random.Random(7)


def health(t, wait):
    out = ask(t, f"data get entity {ME} Health", wait=wait)
    m = re.findall(r"(-?\d+(?:\.\d+)?)[fd]?\]?\s*$", out.strip().splitlines()[-1] if out.strip() else "")
    m = re.findall(r"(\d+\.\d+)f", out)
    return m[-1] if m else out.strip()[-60:]


for t in "KV":
    w(t, "gamerule spawn_mobs false")
    w(t, "difficulty normal")
    w(t, f"gamemode survival {ME}")
    time.sleep(1.0)

for i in range(N):
    pause = rnd.choice([0.0, 0.01, 0.02, 0.03, 0.05, 0.07, 0.1])
    for t in "KV":
        w(t, "gamerule natural_health_regeneration true")
        w(t, f"effect give {ME} minecraft:instant_health 1 10 true")
        w(t, f"effect give {ME} minecraft:saturation 1 10 true")
    time.sleep(1.2)
    for t in "KV":
        # the sequence of command_diff, back to back
        w(t, "gamerule natural_health_regeneration false")
        w(t, f"damage {ME} 2")
        w(t, f"damage {ME} 1 minecraft:nonexistent")
        w(t, "damage Nobody 1")
    time.sleep(pause)
    for t in "KV":
        hist[t][health(t, 0.25)] += 1
    for t in "KV":
        w(t, "gamerule natural_health_regeneration true")
print("K", dict(hist["K"]))
print("V", dict(hist["V"]))
