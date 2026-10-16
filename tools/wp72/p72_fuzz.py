"""wp72: redstone / piston fuzz, Kiln against vanilla 26.3 with real clients.

One process drives both servers (Kiln: console fifo ~/wp72/console, client agent 25999; vanilla: ~/wp72/vanilla/console, client agent 26000).
Per case: a random circuit in a 9x4x9 box on a stone floor, built with setblock while ticking is frozen, then `tick step` to checkpoints;
at every checkpoint both clients dump their view of the box (boxdump) and the two dumps are compared.
usage: p72_fuzz.py SEED0 NCASES [palette]    palette = all | pist | qc | rep
"""
import json, os, random, sys, time
import p61
from p61 import *
from ctl import C, PLAY

p61.VDIR = HOME / "wp72" / "vanilla"
KDIR = PLAY
VDIR = p61.VDIR


def ksv(cmd):
    with open(KDIR / "console", "w") as f:
        f.write(cmd + "\n")


def vsv_(cmd):
    with open(VDIR / "console", "w") as f:
        f.write(cmd + "\n")


def both_(cmd):
    ksv(cmd)
    vsv_(cmd)


DIRS = ["north", "south", "east", "west"]
DIRS6 = DIRS + ["up", "down"]
X0, Y0, Z0 = 10, 200, 10
W, H, D = 9, 4, 9

ka = C(25999)
va = C(26000)


def dump(c):
    return set(json.loads(c.cmd(f"boxdump {X0} {Y0} {Z0} {X0 + W - 1} {Y0 + H} {Z0 + D - 1}")))


def census(c):
    """Counts of the entities of the box by type (positions are not compared: the level random moves items and falling blocks)."""
    out = {}
    for e in c.state(30)["entities"]:
        if X0 - 2 <= e["x"] <= X0 + W + 2 and Y0 - 2 <= e["y"] <= Y0 + H + 40 and Z0 - 2 <= e["z"] <= Z0 + D + 2 and e["t"] != "player":
            out[e["t"]] = out.get(e["t"], 0) + 1
    return tuple(sorted(out.items()))


def stable_dump(c, tries=12):
    prev = (dump(c), census(c))
    for _ in range(tries):
        time.sleep(0.12)
        cur = (dump(c), census(c))
        if cur == prev:
            return cur
        prev = cur
    return prev


def gen(rng, palette):
    cmds = []
    n = rng.randint(8, 34)
    kinds = {
        "all": [("redstone_wire", 6), ("repeater", 3), ("comparator", 2), ("observer", 3), ("piston", 3), ("sticky_piston", 3),
                ("redstone_block", 1), ("slime_block", 2), ("honey_block", 1), ("stone", 3), ("glass", 1), ("obsidian", 1),
                ("redstone_torch", 2), ("redstone_wall_torch", 1), ("redstone_lamp", 1), ("note_block", 1), ("target", 1), ("lever", 1),
                ("hopper", 1), ("sand", 0), ("iron_block", 1)],
        "pist": [("piston", 5), ("sticky_piston", 5), ("observer", 4), ("slime_block", 3), ("honey_block", 2), ("redstone_block", 2),
                 ("stone", 2), ("obsidian", 1), ("glass", 1), ("redstone_wire", 2), ("lever", 1), ("redstone_torch", 1)],
        "qc": [("piston", 4), ("sticky_piston", 3), ("dropper", 3), ("dispenser", 2), ("redstone_wire", 6), ("redstone_block", 2), ("stone", 4),
               ("redstone_torch", 3), ("repeater", 2), ("lever", 2), ("redstone_lamp", 1), ("observer", 1)],
        "dupe": [("piston", 4), ("sticky_piston", 4), ("slime_block", 3), ("honey_block", 1), ("sand", 3), ("gravel", 1), ("rail", 2),
                 ("powered_rail", 2), ("activator_rail", 1), ("white_carpet", 2), ("tnt", 1), ("redstone_block", 2), ("observer", 4),
                 ("redstone_torch", 2), ("hopper", 1), ("chest", 1), ("stone", 2), ("glass", 1), ("lever", 1), ("redstone_wire", 2),
                 ("oak_trapdoor", 1), ("ladder", 1), ("torch", 1), ("flower_pot", 1), ("dirt", 1)],
        "rep": [("repeater", 8), ("comparator", 5), ("redstone_wire", 6), ("redstone_torch", 4), ("stone", 3), ("redstone_block", 1),
                ("lever", 2), ("redstone_lamp", 2), ("observer", 2), ("target", 1)],
    }[palette]
    names = [k for k, w in kinds for _ in range(w)]
    for _ in range(n):
        x, y, z = X0 + rng.randrange(W), Y0 + rng.randrange(H if palette != "rep" else 2), Z0 + rng.randrange(D)
        k = rng.choice(names)
        f = rng.choice(DIRS)
        if k in ("repeater",):
            st = f"repeater[facing={f},delay={rng.randint(1, 4)}]"
        elif k == "comparator":
            st = f"comparator[facing={f},mode={rng.choice(['compare', 'subtract'])}]"
        elif k in ("observer", "dropper", "dispenser"):
            st = f"{k}[facing={rng.choice(DIRS6)}]"
        elif k in ("rail", "powered_rail", "activator_rail"):
            st = f"{k}[shape={rng.choice(['north_south', 'east_west'])}]"
        elif k == "ladder":
            st = f"ladder[facing={f}]"
        elif k == "oak_trapdoor":
            st = f"oak_trapdoor[facing={f},half={rng.choice(['top', 'bottom'])},open={rng.choice(['true', 'false'])}]"
        elif k in ("piston", "sticky_piston"):
            st = f"{k}[facing={rng.choice(DIRS6)}]"
        elif k == "hopper":
            st = f"hopper[facing={rng.choice(DIRS + ['down'])}]"
        elif k == "redstone_wall_torch":
            st = f"redstone_wall_torch[facing={f}]"
        elif k == "lever":
            st = f"lever[face={rng.choice(['floor', 'wall', 'ceiling'])},facing={f}]"
        else:
            st = k
        cmds.append(f"setblock {x} {y} {z} minecraft:{st}")
    # an activator
    x, y, z = X0 + rng.randrange(W), Y0 + rng.randrange(2), Z0 + rng.randrange(D)
    cmds.append(f"setblock {x} {y} {z} minecraft:redstone_block")
    if rng.random() < 0.5:
        x, y, z = X0 + rng.randrange(W), Y0 + rng.randrange(2), Z0 + rng.randrange(D)
        cmds.append(f"setblock {x} {y} {z} minecraft:lever[face=floor,facing=north,powered=true]")
    return cmds


CHECK = [1, 3, 8, 20]
ACT = [0]


def run_case(seed, palette, cmds=None):
    if cmds is None:
        cmds = gen(random.Random(seed), palette)
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 30")
    time.sleep(2.2)  # Kiln steps at the tick rate (30 ticks = 1.5 s); vanilla steps at full speed
    for c in cmds:
        both_(c)
    time.sleep(0.3)
    diffs = []
    t = 0
    first = None
    for step in CHECK:
        both_(f"tick step {step}")
        t += step
        time.sleep(0.3 + 0.06 * step)
        kd, vd = stable_dump(ka), stable_dump(va)
        if first is None:
            first = kd
        ACT[0] += kd != first
        if kd != vd:
            diffs.append((t, sorted(kd[0] - vd[0]) + [f"ents {kd[1]}"], sorted(vd[0] - kd[0]) + [f"ents {vd[1]}"]))
            break
    return cmds, diffs


def minimize(seed, palette):
    """Greedy delta debugging: drop commands while the two servers still differ."""
    cmds, diffs = run_case(seed, palette)
    assert diffs, "no difference to minimise"
    changed = True
    while changed:
        changed = False
        for i in range(len(cmds) - 1, -1, -1):
            trial = cmds[:i] + cmds[i + 1:]
            _, d = run_case(seed, palette, trial)
            if d:
                cmds, diffs, changed = trial, d, True
    print("MINIMAL", len(cmds), "commands, tick", diffs[0][0], flush=True)
    for c in cmds:
        print("   ", c)
    print("   only K:", diffs[0][1][:10])
    print("   only V:", diffs[0][2][:10], flush=True)


def setup_world():
    vsv_("op KilnTest")
    for g in ("random_tick_speed 0", "spawn_mobs false", "do_weather_cycle false", "do_daylight_cycle false", "mob_griefing false"):
        both_("gamerule " + g)
    p61.sv("forceload add 0 0 40 40", show=False)
    p61.vsv("forceload add 0 0 40 40", show=False)
    both_("tick freeze")


def main():
    if sys.argv[1] == "min":
        setup_world()
        minimize(int(sys.argv[2]), sys.argv[3])
        return
    seed0, n = int(sys.argv[1]), int(sys.argv[2])
    palette = sys.argv[3] if len(sys.argv) > 3 else "all"
    vsv_("op KilnTest")
    for g in ("random_tick_speed 0", "spawn_mobs false", "do_weather_cycle false", "do_daylight_cycle false", "mob_griefing false"):
        both_("gamerule " + g)
    p61.sv("forceload add 0 0 40 40", show=False)
    p61.vsv("forceload add 0 0 40 40", show=False)
    both_("tick freeze")
    bad = 0
    out = open(HOME / "wp72" / f"fuzz_{palette}_{seed0}.txt", "w")
    for seed in range(seed0, seed0 + n):
        cmds, diffs = run_case(seed, palette)
        if diffs:
            bad += 1
            t, kmv, vmk = diffs[0]
            msg = f"DIFF seed={seed} palette={palette} tick={t}\n  only K: {kmv[:8]}\n  only V: {vmk[:8]}\n  cmds: {cmds}\n"
            print(msg, flush=True)
            out.write(msg)
            out.flush()
        elif seed % 10 == 0:
            print("ok through", seed, flush=True)
    print(f"done: {n} cases, {bad} differ, {ACT[0]} checkpoints differed from the first one (activity)", flush=True)
    both_("tick unfreeze")


main()
