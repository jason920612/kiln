"""wp65: sugar cane and kelp farms with observers and pistons, built with commands (survival-style: an observer watching the plant, a stone,
dust, a stone and a piston breaking it), run with random_tick_speed up and the server fast; the harvested items are counted.
Kiln (TARGET=K) vs vanilla (TARGET=V).   usage: p65_farms.py SECONDS"""
from p65_b2 import *
import sys, os

secs = int(sys.argv[1]) if len(sys.argv) > 1 else 60
X0, Z0, Y = int(os.environ.get("FX") or "5000"), int(os.environ.get("FZ") or "5000"), 200
N = 8


def build():
    sv(f"forceload add {X0-4} {Z0-4} {X0+8} {Z0+4*N+4}", show=False)
    time.sleep(3)
    for _ in range(40):
        out = sv(f"setblock {X0} {Y} {Z0} minecraft:stone", show=False, wait=0.8)
        out += sv(f"setblock {X0+8} {Y} {Z0+4*N+4} minecraft:stone", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv(f"fill {X0-4} {Y-1} {Z0-4} {X0+8} {Y+10} {Z0+4*N+4} minecraft:air", show=False)
    sv(f"fill {X0-4} {Y-1} {Z0-4} {X0+8} {Y-1} {Z0+4*N+4} minecraft:stone", show=False)
    for i in range(N):
        z = Z0 + 4 * i
        if i % 2 == 0:
            # sugar cane: sand at x0, water at x0+1, cane two blocks high, observer on top looking down, stone, stone + dust, piston facing east
            sv(f"setblock {X0} {Y} {z} minecraft:sand", show=False)
            sv(f"setblock {X0+1} {Y} {z} minecraft:water", show=False)
            sv(f"setblock {X0+1} {Y+1} {z} minecraft:stone", show=False)  # keeps the water from spilling
            sv(f"setblock {X0+2} {Y} {z} minecraft:stone", show=False)
            sv(f"setblock {X0} {Y+1} {z} minecraft:sugar_cane", show=False)
            sv(f"setblock {X0-1} {Y+2} {z} minecraft:piston[facing=east]", show=False)
            sv(f"setblock {X0} {Y+3} {z} minecraft:observer[facing=down]", show=False)
            sv(f"setblock {X0} {Y+4} {z} minecraft:stone", show=False)
            sv(f"setblock {X0-1} {Y+3} {z} minecraft:stone", show=False)
            sv(f"setblock {X0-1} {Y+4} {z} minecraft:redstone_wire", show=False)
            sv(f"fill {X0+1} {Y} {z-1} {X0+1} {Y} {z-1} minecraft:stone", show=False)
            sv(f"fill {X0+1} {Y} {z+1} {X0+1} {Y} {z+1} minecraft:stone", show=False)
        else:
            # kelp: a water column on sand, kelp grows up it; observer on top looking down, the same chain, the piston in the water
            sv(f"fill {X0} {Y+1} {z} {X0} {Y+3} {z} minecraft:water", show=False)
            sv(f"setblock {X0} {Y} {z} minecraft:sand", show=False)
            sv(f"setblock {X0} {Y+1} {z} minecraft:kelp[age=0]", show=False)
            sv(f"fill {X0+1} {Y} {z} {X0+1} {Y+4} {z} minecraft:stone", show=False)
            sv(f"fill {X0} {Y+1} {z-1} {X0} {Y+4} {z-1} minecraft:stone", show=False)
            sv(f"fill {X0} {Y+1} {z+1} {X0} {Y+4} {z+1} minecraft:stone", show=False)
            sv(f"setblock {X0-1} {Y+3} {z} minecraft:piston[facing=east]", show=False)
            sv(f"setblock {X0} {Y+4} {z} minecraft:observer[facing=down]", show=False)
            sv(f"setblock {X0} {Y+5} {z} minecraft:stone", show=False)
            sv(f"setblock {X0-1} {Y+4} {z} minecraft:stone", show=False)
            sv(f"setblock {X0-1} {Y+5} {z} minecraft:redstone_wire", show=False)


def items():
    out = sv(f"execute as @e[type=minecraft:item,x={X0-4},y={Y-3},z={Z0-4},dx=14,dy=16,dz={4*N+10}] run data get entity @s Item", show=False, wait=3.0)
    tot = {}
    for comp in re.findall(r"\{[^{}]*\}", out):
        c = re.search(r"count: ([0-9]+)", comp)
        i = re.search(r'id: "minecraft:([a-z_]+)"', comp)
        if c and i:
            tot[i.group(1)] = tot.get(i.group(1), 0) + int(c.group(1))
    return tot


def farms():
    prep("creative")
    sv("gamerule spawn_mobs false", show=False)
    sv("kill @e[type=!player]", show=False)
    build()
    sv("gamerule random_tick_speed 2000", show=False)
    for nm in ("KilnTest", "Obs"):
        sv(f"gamemode creative {nm}", show=False)
        px, pz = int(os.environ.get("PX") or "3"), int(os.environ.get("PZ") or "16")
        sv(f"fill {X0 + px - 2} {Y - 1} {Z0 + pz - 2} {X0 + px + 2} {Y - 1} {Z0 + pz + 2} minecraft:stone", show=False)
        sv(f"tp {nm} {X0 + px}.5 {Y} {Z0 + pz}.5", show=False)
    time.sleep(6)
    F("built; cane at", a.block(X0, Y + 1, Z0), "piston", a.block(X0 - 1, Y + 1, Z0))
    def gt():
        r = sv("time query gametime", show=False)
        m = re.search(r"time is ([0-9]+)", r) or re.search(r"gametime\[([0-9]+)\]", r)
        return int(m.group(1)) if m else -1
    g0 = gt()
    sv("tick rate 100", show=False)
    t0 = time.time()
    while time.time() - t0 < secs:
        time.sleep(15)
        F("t+%d s items %s" % (time.time() - t0, items()), "|", mspt()[:70])
    sv("tick rate 20", show=False)
    sv("gamerule random_tick_speed 3", show=False)
    g1 = gt()
    F("game ticks elapsed", g1 - g0, "(%.0f tps)" % ((g1 - g0) / (time.time() - t0)))
    F("final items", items())
    shot("farms")


if __name__ == "__main__":
    farms()
