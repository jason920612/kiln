"""wp81: how far from a player does each kind of ticking reach (in chunks east of the player's chunk), Kiln (K) against vanilla (V)? Probes, per distance d = 0..9:
  age:    a baby cow (Age -24000, NoGravity) in the middle of chunk d: its Age rises 1 per tick if the chunk entity-ticks (AgeableMob.aiStep);
  water:  a water source placed on a stone floor in chunk d spreads (a scheduled fluid tick, which runs where `shouldTickBlocksAt`);
  growth: a sapling-like random tick is not probed here (random ticks are `forEachBlockTickingChunk` -> the entity-ticking chunks).
Both servers: seed 77, view distance VIEW, simulation distance SIM (sv.sh), the player W0 in creative at (X0+8.5, 220, Z0+8.5). usage: simradius81.py [TICKS]"""
import re, sys, time
from lk import *

TICKS = int(sys.argv[1]) if len(sys.argv) > 1 else 40
X0, Z0, Y = 1000, 1000, 220


def ages(t):
    res = {}
    for d in range(0, 10):
        out = ask(t, f"data get entity @e[tag=rt{d},limit=1] Age", wait=0.2)
        m2 = re.findall(r"(-?\d+)(?!.*\d)", out)
        res[d] = m2[-1] if m2 else None
    return res


def loaded(t):
    res = {}
    for d in range(0, 14):
        x = X0 + 16 * d + 8
        out = ask(t, f"execute if loaded {x} {Y - 41} {Z0 + 8}", wait=0.15)
        res[d] = ("pass" in out.lower())
    return res


def source(t):
    res = {}
    for d in range(0, 14):
        x = X0 + 16 * d + 8
        out = ask(t, f"execute if block {x} {Y - 41} {Z0 + 8} minecraft:water", wait=0.15)
        res[d] = ("pass" in out.lower())
    return res


def water(t):
    res = {}
    for d in range(0, 10):
        x = X0 + 16 * d + 8
        out = ask(t, f"execute if block {x + 1} {Y - 41} {Z0 + 8} minecraft:water", wait=0.15)
        res[d] = ("pass" in out.lower())
    return res


for t in "KV":
    w(t, "gamerule spawn_mobs false")
    w(t, "gamerule advance_time false")
    w(t, "gamemode creative W0")
    w(t, f"tp W0 {X0 + 8.5} {Y} {Z0 + 8.5}")
time.sleep(25)
for t in "KV":
    w(t, "kill @e[type=minecraft:cow]")
    for d in range(10):
        w(t, f"summon cow {X0 + 8.5 + 16 * d} {Y - 30} {Z0 + 8.5} {{Age:-24000,NoGravity:1b,PersistenceRequired:1b,Tags:[\"rt{d}\"]}}")
        x = X0 + 16 * d + 8
        w(t, f"fill {x - 2} {Y - 42} {Z0 + 6} {x + 3} {Y - 42} {Z0 + 10} minecraft:stone")
        w(t, f"fill {x - 2} {Y - 41} {Z0 + 6} {x + 3} {Y - 38} {Z0 + 10} minecraft:glass")
        w(t, f"fill {x - 1} {Y - 41} {Z0 + 7} {x + 2} {Y - 41} {Z0 + 9} minecraft:air")
        w(t, f"setblock {x} {Y - 41} {Z0 + 8} minecraft:water")
time.sleep(0.5)
before = {t: ages(t) for t in "KV"}
for t in "KV":
    w(t, f"tick sprint {TICKS}")
time.sleep(0.5 + TICKS * 0.06)
time.sleep(2)
after = {t: ages(t) for t in "KV"}
spread = {t: water(t) for t in "KV"}
isloaded = {t: loaded(t) for t in "KV"}
placed = {t: source(t) for t in "KV"}
print("distance east (chunks): Age before -> after; the water source spread to its neighbour")
for d in range(10):
    print(d, "K", before["K"][d], "->", after["K"][d], spread["K"][d], "  V", before["V"][d], "->", after["V"][d], spread["V"][d])
print("execute if loaded (entity ticking), distance 0..13:")
for t in "KV":
    print(t, "".join("1" if isloaded[t][d] else "0" for d in range(14)))
print("water source placed (the chunk was loaded when the probes went in), distance 0..13:")
for t in "KV":
    print(t, "".join("1" if placed[t][d] else "0" for d in range(14)))
