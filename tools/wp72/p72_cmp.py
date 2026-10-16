"""wp72: comparator readings of containers and other blocks, Kiln against vanilla (one process, both consoles, frozen).
For every case a block is set up (items in random slots / a state) with a comparator reading it (directly or through a solid block) and
the comparator's OutputSignal is read from both servers after the comparator's delay. usage: p72_cmp.py SEED N"""
import random, sys, re
from lock import *

X0, Y, Z0 = 10, 200, 10
ITEMS = [("minecraft:cobblestone", 64), ("minecraft:ender_pearl", 16), ("minecraft:snowball", 16), ("minecraft:diamond_sword", 1), ("minecraft:egg", 16),
         ("minecraft:bucket", 16), ("minecraft:water_bucket", 1), ("minecraft:white_shulker_box", 1), ("minecraft:sign", 16), ("minecraft:oak_log", 64), ("minecraft:stick", 64),
         ("minecraft:iron_ingot", 64), ("minecraft:music_disc_13", 1), ("minecraft:potion", 1), ("minecraft:writable_book", 1), ("minecraft:armor_stand", 16)]


def stack(slot, rng, only=None):
    item, mx = rng.choice(ITEMS) if only is None else only
    n = rng.choice([1, 2, 3, mx // 2 or 1, mx - 1 or 1, mx])
    n = max(1, min(n, mx))
    return f'{{Slot:{slot}b,id:"{item}",count:{n}}}'


def items_nbt(size, rng, fill, only=None):
    slots = [s for s in range(size) if rng.random() < fill]
    return "{Items:[" + ",".join(stack(s, rng, only) for s in slots) + "]}"


def case(name, block, nbt, rng, via_solid=False, facing="east"):
    """A comparator at x+1 (or x+2 behind a stone) facing away from the block."""
    return name, block, nbt, via_solid


CONTAINERS = [
    ("chest", "minecraft:chest", 27), ("trapped_chest", "minecraft:trapped_chest", 27), ("barrel", "minecraft:barrel", 27),
    ("hopper", "minecraft:hopper", 5), ("dropper", "minecraft:dropper", 9), ("dispenser", "minecraft:dispenser", 9),
    ("furnace", "minecraft:furnace", 3), ("blast_furnace", "minecraft:blast_furnace", 3), ("smoker", "minecraft:smoker", 3),
    ("brewing_stand", "minecraft:brewing_stand", 5), ("shulker_box", "minecraft:shulker_box", 27), ("crafter", "minecraft:crafter", 9),
]


def build(rng):
    """Returns (description, setup commands, comparator position)."""
    kind = rng.random()
    cx, cy, cz = X0 + 3 * rng.randrange(6), Y, Z0 + 3 * rng.randrange(6)
    comp_pos = (cx + 1, cy, cz)
    cmds = [f"fill {cx - 1} {cy - 1} {cz - 1} {cx + 3} {cy + 2} {cz + 1} minecraft:air", f"fill {cx - 1} {cy - 1} {cz - 1} {cx + 3} {cy - 1} {cz + 1} minecraft:stone"]
    desc = ""
    comp = "minecraft:comparator[facing=east,mode={mode}]"
    mode = rng.choice(["compare", "compare", "subtract"])
    if kind < 0.6:
        name, block, size = rng.choice(CONTAINERS)
        fill = rng.choice([0.0, 0.05, 0.2, 0.5, 0.8, 1.0])
        nbt = items_nbt(size, rng, fill)
        if name == "brewing_stand":
            nbt = nbt  # potions as a brewing stand holds them: any item
        cmds.append(f"setblock {cx} {cy} {cz} {block}{nbt}")
        desc = f"{name} size {size} fill {fill} {nbt[:80]}"
    elif kind < 0.7:
        # a double chest: two halves with items
        a = items_nbt(27, rng, rng.choice([0.1, 0.5, 1.0]))
        b = items_nbt(27, rng, rng.choice([0.1, 0.5, 1.0]))
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:chest[type=left,facing=south]{a}")
        cmds.append(f"setblock {cx} {cy} {cz + 1} minecraft:chest[type=right,facing=south]{b}")
        cmds.append(f"fill {cx - 1} {cy} {cz + 1} {cx - 1} {cy} {cz + 1} minecraft:stone")
        desc = f"double chest {a[:40]} | {b[:40]}"
    elif kind < 0.78:
        lvl = rng.randrange(9)
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:composter[level={lvl}]")
        desc = f"composter level {lvl}"
    elif kind < 0.82:
        bites = rng.randrange(7)
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:cake[bites={bites}]")
        desc = f"cake bites {bites}"
    elif kind < 0.88:
        liquid = rng.choice(["water_cauldron[level=%d]" % rng.randint(1, 3), "lava_cauldron", "powder_snow_cauldron[level=%d]" % rng.randint(1, 3), "cauldron"])
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:{liquid}")
        desc = liquid
    elif kind < 0.92:
        lvl = rng.randrange(6)
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:beehive[honey_level={lvl}]")
        desc = f"beehive honey {lvl}"
    elif kind < 0.95:
        ch = rng.randrange(5)
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:respawn_anchor[charges={ch}]")
        desc = f"respawn anchor {ch}"
    elif kind < 0.97:
        eye = rng.choice(["true", "false"])
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:end_portal_frame[eye={eye},facing=north]")
        desc = f"end portal frame eye {eye}"
    else:
        d = rng.choice(["compare"])
        cmds.append(f"setblock {cx} {cy} {cz} minecraft:jukebox{{RecordItem:{{id:\"minecraft:music_disc_13\",count:1}}}}")
        desc = "jukebox with a disc"
    through = rng.random() < 0.25
    if through:
        cmds.append(f"setblock {cx + 1} {cy} {cz} minecraft:{rng.choice(['stone', 'glass', 'oak_planks', 'iron_block'])}")
        comp_pos = (cx + 2, cy, cz)
        desc += " (through a block)"
    cmds.append(f"setblock {comp_pos[0]} {comp_pos[1]} {comp_pos[2]} " + comp.format(mode=mode))
    if rng.random() < 0.3:
        # a second signal from the side for the subtract/compare modes
        side = rng.randint(0, 15)
        cmds.append(f"setblock {comp_pos[0]} {comp_pos[1]} {comp_pos[2] + 1} minecraft:stone")
        cmds.append(f"setblock {comp_pos[0]} {comp_pos[1] + 1} {comp_pos[2] + 1} minecraft:redstone_wire[power={side}]")
        desc += f" side signal {side}"
    return desc + f" mode={mode}", cmds, comp_pos


def signal(t, pos):
    out = ask(t, f"data get block {pos[0]} {pos[1]} {pos[2]} OutputSignal", wait=0.25)
    nums = re.findall(r"(-?\d+)\b", out)
    return nums[-1] if nums else out.strip()[:60]


def main():
    seed0, n = int(sys.argv[1]), int(sys.argv[2])
    setup()
    bad = 0
    for seed in range(seed0, seed0 + n):
        rng = random.Random(seed)
        desc, cmds, pos = build(rng)
        for c in cmds:
            both_(c)
        time.sleep(0.3)
        step(4)
        k, v = signal("K", pos), signal("V", pos)
        ok = k == v
        if not ok:
            bad += 1
        print(("ok  " if ok else "DIFF"), seed, k, v, desc[:160], flush=True)
        if not ok:
            print("   ", cmds, flush=True)
    print(f"done: {n} cases, {bad} differ")
    both_("tick unfreeze")


main()
