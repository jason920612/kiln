"""wp82 item 1: raids on Kiln (K) and vanilla (V) at the same village (seed 78, the bell of the village at 300,188), each server on its own thread in
real time. The player (bot W0, creative) stands at the bell; bad omen -> raid omen -> raid; then the raiders are killed after they walked in a while and
the next wave comes (WAVES waves): per wave the seconds from the bar filling to the raiders existing, and their distances from the player when they first
exist. usage: raid82.py [waves] [amplifier]"""
import threading
from lab import *
WAVES = int(sys.argv[1]) if len(sys.argv) > 1 else 3
AMP = int(sys.argv[2]) if len(sys.argv) > 2 else 1
VX, VZ = 300, 188
res = {"K": [], "V": []}
NUM = r"(-?\d+(?:\.\d+)?)d?"

def positions(t):
    o = ask(t, "execute as @e[type=#minecraft:raiders] run data get entity @s Pos", wait=0.5)
    out = []
    for m in re.finditer(r"\[" + NUM + r", " + NUM + r", " + NUM + r"\]", o):
        out.append(tuple(float(m.group(i)) for i in (1, 2, 3)))
    return out

def run(t):
    w(t, "kill @e[type=#minecraft:raiders]"); w(t, "effect clear W0"); time.sleep(1)
    for g in ("raids true", "spawn_mobs false", "advance_time false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "time set day"); w(t, "gamemode creative W0")
    w(t, f"execute in minecraft:overworld run forceload add {VX - 48} {VZ - 48} {VX + 48} {VZ + 48}")
    time.sleep(2)
    w(t, f"execute in minecraft:overworld run tp W0 {VX}.5 150 {VZ}.5")
    time.sleep(25)
    w(t, f"execute in minecraft:overworld positioned {VX}.5 0 {VZ}.5 positioned over motion_blocking_no_leaves run tp W0 ~ ~ ~")
    time.sleep(3)
    me = positions_of(t)
    w(t, f"effect give W0 minecraft:bad_omen 6000 {AMP} true")
    t0 = time.time()
    for wave in range(WAVES):
        seen = None
        while time.time() - t0 < 400:
            ps = positions(t)
            if ps:
                seen = (round(time.time() - t0, 1), len(ps), sorted(round(((p[0] - me[0]) ** 2 + (p[2] - me[2]) ** 2) ** 0.5) for p in ps))
                res[t].append((wave + 1, *seen))
                break
            time.sleep(0.2)
        time.sleep(25)
        w(t, "kill @e[type=#minecraft:raiders]")
        while positions(t):
            time.sleep(0.2)
        t0 = time.time()
    w(t, "effect clear W0")

def positions_of(t):
    o = ask(t, "data get entity W0 Pos", wait=1)
    m = re.search(r"\[" + NUM + r", " + NUM + r", " + NUM + r"\]", o)
    return tuple(float(m.group(i)) for i in (1, 2, 3))

ths = [threading.Thread(target=run, args=(t,)) for t in "KV"]
[th.start() for th in ths]
[th.join() for th in ths]
for t in "KV":
    F("==", t, "(wave, seconds since the omen/the last wave was cleared, how many, their distances from the player when first seen)")
    for r in res[t]:
        F("  ", r)
