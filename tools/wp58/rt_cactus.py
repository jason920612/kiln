"""wp66 item 3: the random-tick sampling seen through cactus age counters. A template section (sand/cactus on a 2-grid, 4 layers, 256
cacti) is cloned into NC chunks; after `tick step` each cactus's age is the number of random ticks it got (up to 15).
usage: TARGET=K|V rt_cactus.py SPEED STEPS REPS [NCX NCZ]  -> histogram of ages after STEPS ticks over REPS clones."""
import sys, math
from p66rt import *

speed, steps, reps = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
NCX, NCZ = (int(sys.argv[4]), int(sys.argv[5])) if len(sys.argv) > 5 else (4, 4)
BX, BZ = 1024, 1024


def setup():
    run([f"forceload add 0 0 0 0", f"forceload add {BX} {BZ} {BX+16*NCX-1} {BZ+16*NCZ-1}"])
    assert ready(0, 0) and ready(BX, BZ) and ready(BX+16*NCX-1, BZ+16*NCZ-1)
    run(["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_weather false", "gamerule advance_time false", "time set day", "weather clear"])
    cmds = ["fill 0 60 0 15 80 15 air"]
    for k in range(4):
        y = 64 + 4 * k
        for i in range(8):
            for j in range(8):
                cmds.append(f"setblock {2*i} {y} {2*j} minecraft:stone")
                cmds.append(f"setblock {2*i} {y+1} {2*j} minecraft:sand")
                cmds.append(f"setblock {2*i} {y+2} {2*j} minecraft:cactus")
    run(cmds)
    # cactus survives only with no solid horizontal neighbours: check the template stayed
    n = counts(["fill 0 60 0 15 80 15 minecraft:cactus[age=1] replace minecraft:cactus[age=0]", "fill 0 60 0 15 80 15 minecraft:cactus[age=0] replace minecraft:cactus[age=1]"])
    return n


def clone_all():
    return [f"clone 0 64 0 15 79 15 {BX+16*i} 64 {BZ+16*j}" for i in range(NCX) for j in range(NCZ)]


def read_ages():
    """Count cacti per age over the whole area (each chunk, ages 0..14; 15 and the total by subtraction)."""
    cmds = []
    for i in range(NCX):
        for j in range(NCZ):
            x, z = BX + 16 * i, BZ + 16 * j
            for k in range(15):
                cmds.append(f"fill {x} 64 {z} {x+15} 79 {z+15} minecraft:cactus[age=15] replace minecraft:cactus[age={k}]")
    res = counts(cmds)
    assert len(res) == len(cmds), (len(res), len(cmds))
    hist = [0] * 16
    idx = 0
    for _ in range(NCX * NCZ):
        for k in range(15):
            hist[k] += res[idx]
            idx += 1
    total = NCX * NCZ * 256
    hist[15] = total - sum(hist[:15])
    return hist


setup_n = setup()
print("template cacti:", setup_n, flush=True)
tot = [0] * 16
t0 = time.time()
for r in range(reps):
    run(clone_all())
    run([f"gamerule random_tick_speed {speed}"])
    step(steps)
    run([f"gamerule random_tick_speed 0"])
    h = read_ages()
    tot = [a + b for a, b in zip(tot, h)]
n = sum(tot)
mean = sum(k * v for k, v in enumerate(tot)) / n
var = sum((k - mean) ** 2 * v for k, v in enumerate(tot)) / n
print(f"TARGET={T} speed={speed} steps={steps} reps={reps} positions={n} mean {mean:.4f} (expected {steps*speed/4096:.4f}) var {var:.4f} (poisson {mean:.4f})  [{time.time()-t0:.0f}s]")
print("hist", tot)
