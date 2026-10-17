"""wp74: redstone / piston fuzz, Kiln against vanilla 26.3 with real clients.

One process drives both servers (Kiln: console fifo ~/wp74/console, client agent 25999; vanilla: ~/wp74/vanilla/console, client agent 26000).
Per case: a random circuit in a 9x4x9 box on a stone floor, built with setblock while ticking is frozen, then `tick step` to checkpoints;
at every checkpoint both clients dump their view of the box (boxdump) and the two dumps are compared.
usage: p74_fuzz.py SEED0 NCASES [palette]    palette = all | pist | qc | rep
"""
import json, os, random, re, sys, time
from lock import w, logpos, since
import p61
from p61 import *
from ctl import C, PLAY

p61.VDIR = HOME / "wp74" / "vanilla"
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
        if X0 - 2 <= e["x"] <= X0 + W + 2 and Y0 - 2 <= e["y"] <= Y0 + H + 40 and Z0 - 2 <= e["z"] <= Z0 + D + 2 and e["t"] not in ("player", "item", "experience_orb"):  # items merge, move and get picked up with the level random: not compared
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
    if palette == "dupe2":
        return gen2(rng)
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


NEIGH = [(dx, dy, dz) for dx in (-1, 0, 1, 2) for dy in range(0, 5) for dz in (-1, 0, 1)]


def gen2(rng):
    """A slime column (2-4 high) pushed by a sticky piston clocked by two observers, with random carpets / coral fans / rails / TNT / more slime around it:
    the shapes of the known piston-based carpet, rail and TNT duplicators, in random variations."""
    cmds = []
    ox, oz = X0 + 3 + rng.randrange(3), Z0 + 3 + rng.randrange(3)
    h = rng.randint(2, 4)
    used = set()

    def put(x, y, z, st):
        if (x, y, z) in used:
            return
        used.add((x, y, z))
        cmds.append(f"setblock {x} {y} {z} minecraft:{st}")
    for k in range(h):
        put(ox, Y0 + k, oz, "slime_block")
    d = rng.choice(["east", "west", "north", "south"])
    vx, vz = {"east": (1, 0), "west": (-1, 0), "north": (0, -1), "south": (0, 1)}[d]
    py = Y0 + rng.randrange(h)
    put(ox - vx, py, oz - vz, f"{rng.choice(['sticky_piston', 'sticky_piston', 'piston'])}[facing={d}]")
    # the clock: two observers facing each other next to the piston, or a lever / redstone block
    kind = rng.choice(["obs", "obs", "block", "lever"])
    if kind == "obs":
        bx, bz = ox - 2 * vx, oz - 2 * vz
        put(bx, py, bz, f"observer[facing={d}]")
        put(bx - vx, py, bz - vz, f"observer[facing={ {'east': 'west', 'west': 'east', 'north': 'south', 'south': 'north'}[d] }]")
    elif kind == "block":
        put(ox - vx, py + 1, oz - vz, "redstone_block")
    else:
        put(ox - vx - vz, py, oz - vz + vx, "stone")
        put(ox - vx - vz, py + 1, oz - vz + vx, "lever[face=floor,facing=north,powered=true]")
    names = ["white_carpet", "white_carpet", "brain_coral_fan", "tube_coral_fan", "rail", "powered_rail", "detector_rail", "activator_rail", "tnt", "tnt",
             "slime_block", "slime_block", "redstone_block", "stone", "observer", "sticky_piston", "piston", "honey_block", "oak_trapdoor", "oak_button", "redstone_wire", "hopper"]
    for _ in range(rng.randint(3, 11)):
        dx, dy, dz = rng.choice(NEIGH)
        x, y, z = ox + dx, Y0 + dy, oz + dz
        if not (X0 <= x < X0 + W and Z0 <= z < Z0 + D):
            continue
        k = rng.choice(names)
        if k in ("observer", "sticky_piston", "piston"):
            st = f"{k}[facing={rng.choice(DIRS6)}]"
        elif k in ("rail", "powered_rail", "detector_rail", "activator_rail"):
            st = f"{k}[shape={rng.choice(['north_south', 'east_west'])}]"
        elif k == "oak_trapdoor":
            st = f"oak_trapdoor[facing={rng.choice(DIRS)},half={rng.choice(['top', 'bottom'])},open=false]"
        elif k == "oak_button":
            st = f"oak_button[face=floor,facing=north]"
        elif k == "hopper":
            st = f"hopper[facing={rng.choice(DIRS + ['down'])}]"
        else:
            st = k
        put(x, y, z, st)
    if rng.random() < 0.5:
        dx, dy, dz = rng.choice(NEIGH)
        x, y, z = ox + dx, Y0 + dy, oz + dz
        if X0 <= x < X0 + W and Z0 <= z < Z0 + D:
            cmds.append(f"summon minecraft:minecart {x}.5 {y}.1 {z}.5")
    return cmds


CHECK = [int(x) for x in os.environ.get('P72_CHECK', '1,3,8,20').split(',')]  # steps between the checkpoints (1,1,1,... = every tick)
ACT = [0]


def run_case(seed, palette, cmds=None):
    if cmds is None:
        cmds = gen(random.Random(seed), palette)
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 30")
    time.sleep(2.2)
    for _ in range(5):
        # nothing of the last case may be left in the box (items, falling blocks): the census counts them
        if not census(ka) and not census(va):
            break
        both_("kill @e[type=!player]")
        both_("tick step 3")
        time.sleep(0.8)
    time.sleep(0.01)  # Kiln steps at the tick rate (30 ticks = 1.5 s); vanilla steps at full speed
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
    if not diffs:
        # the dupes themselves: item entities and TNT left in the box
        ik = (count_items(ka_t := "K"), count_items("V"))
        if ik[0] != ik[1]:
            diffs.append((t, [f"item total K {ik[0]}"], [f"item total V {ik[1]}"]))
    return cmds, diffs


def count_items(t):
    """The total stack count of the item entities of the box (merging and despawn timing do not matter, a duplicated item does)."""
    sel = f"@e[type=item,x={X0 - 3},y={Y0 - 3},z={Z0 - 3},dx={W + 6},dy={H + 80},dz={D + 6}]"
    n = logpos(t)
    w(t, f"execute as {sel} run data get entity @s Item.count")
    time.sleep(0.8)
    txt = since(t, n)
    tot = 0
    for line in txt.splitlines():
        m = re.search(r"query\[[^,\]]+, (\d+)\]|entity data: (\d+)\s*$", line)
        if m:
            tot += int(m.group(1) or m.group(2))
    return tot


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
    out = open(HOME / "wp74" / f"fuzz_{palette}_{seed0}.txt", "w")
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
