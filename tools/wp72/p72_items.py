"""wp72: item and entity physics in technical builds, Kiln against vanilla, tick by tick in lock step. Items are summoned with an explicit
Motion (no level random), tagged, and their Pos and Motion read every tick from both servers (`data get entity`); the first tick at which
the two differ is reported. Also container contents after hopper chains. usage: p72_items.py [scenario ...] (default: all)"""
import re, sys
from lock import *

X0, Y0, Z0 = 12, 200, 12
DBL = re.compile(r"(-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?)[dfD]")


def item(tag, x, y, z, mot=(0, 0, 0), stack="minecraft:stone", pickup=32767):
    return (f'summon minecraft:item {x} {y} {z} {{Item:{{id:"{stack}",count:1}},Motion:[{mot[0]}d,{mot[1]}d,{mot[2]}d],PickupDelay:{pickup}s,Tags:["{tag}"]}}')


def read(t, tag, path):
    out = ask(t, f"data get entity @e[tag={tag},limit=1] {path}", wait=0.22)
    return DBL.findall(out.split("data:")[-1] if "data:" in out else out[out.find("["):]) or ["gone"]


def clear(x0=X0 - 2, z0=Z0 - 2, x1=X0 + 30, z1=Z0 + 12):
    both_(f"fill {x0} {Y0 - 2} {z0} {x1} {Y0 + 12} {z1} minecraft:air")
    both_(f"fill {x0} {Y0 - 1} {z0} {x1} {Y0 - 1} {z1} minecraft:stone")
    both_("kill @e[type=!player]")
    step(5)
    time.sleep(1.0)


def watch(name, tags, ticks, every=1):
    """Steps `ticks` ticks and compares Pos and Motion of each tag on the two servers after every `every` ticks."""
    first = None
    for k in range(1, ticks + 1):
        step(1)
        if k % every:
            continue
        for tag in tags:
            for path in ("Pos", "Motion"):
                a, b = read("K", tag, path), read("V", tag, path)
                if a != b and first is None:
                    first = (k, tag, path, a, b)
    print(("SAME  " if first is None else "DIFF  ") + name, first if first else "", flush=True)
    return first


def scenarios():
    S = {}

    def water_channel():
        clear()
        both_(f"fill {X0} {Y0} {Z0} {X0 + 14} {Y0} {Z0} minecraft:air")
        both_(f"fill {X0 - 1} {Y0} {Z0 - 1} {X0 + 15} {Y0 + 1} {Z0 - 1} minecraft:glass")
        both_(f"fill {X0 - 1} {Y0} {Z0 + 1} {X0 + 15} {Y0 + 1} {Z0 + 1} minecraft:glass")
        both_(f"setblock {X0} {Y0} {Z0} minecraft:water")
        step(70)
        time.sleep(2)
        tags = []
        for i, (dx, dy) in enumerate([(2.5, 0.2), (3.5, 0.6), (5.5, 0.1), (8.5, 0.9)]):
            both_(item(f"w{i}", X0 + dx, Y0 + dy, Z0 + 0.5))
            tags.append(f"w{i}")
        return tags, 120

    def bubble_up():
        clear()
        both_(f"fill {X0} {Y0} {Z0} {X0} {Y0 + 8} {Z0} minecraft:water")
        both_(f"setblock {X0} {Y0 - 1} {Z0} minecraft:soul_sand")
        step(30)
        time.sleep(1)
        both_(item("b0", X0 + 0.5, Y0 + 3.0, Z0 + 0.5))
        both_(item("b1", X0 + 0.5, Y0 + 0.2, Z0 + 0.5, mot=(0.03, 0.0, 0.0)))
        return ["b0", "b1"], 100

    def bubble_down():
        clear()
        both_(f"fill {X0} {Y0} {Z0} {X0} {Y0 + 8} {Z0} minecraft:water")
        both_(f"setblock {X0} {Y0 - 1} {Z0} minecraft:magma_block")
        step(30)
        time.sleep(1)
        both_(item("d0", X0 + 0.5, Y0 + 6.0, Z0 + 0.5))
        both_(item("d1", X0 + 0.5, Y0 + 2.2, Z0 + 0.5))
        return ["d0", "d1"], 100

    def ground_slide():
        clear()
        both_(f"fill {X0} {Y0 - 1} {Z0} {X0 + 4} {Y0 - 1} {Z0} minecraft:ice")
        both_(f"fill {X0 + 5} {Y0 - 1} {Z0} {X0 + 9} {Y0 - 1} {Z0} minecraft:blue_ice")
        both_(f"fill {X0 + 10} {Y0 - 1} {Z0} {X0 + 14} {Y0 - 1} {Z0} minecraft:slime_block")
        both_(f"fill {X0 + 15} {Y0 - 1} {Z0} {X0 + 19} {Y0 - 1} {Z0} minecraft:honey_block")
        both_(f"fill {X0 + 20} {Y0 - 1} {Z0} {X0 + 24} {Y0 - 1} {Z0} minecraft:soul_sand")
        tags = []
        for i, dx in enumerate((0.5, 5.5, 10.5, 15.5, 20.5)):
            both_(item(f"g{i}", X0 + dx, Y0 + 0.5, Z0 + 0.5, mot=(0.2, 0.15, 0)))
            tags.append(f"g{i}")
        return tags, 80

    def fall_and_lava():
        clear()
        both_(f"fill {X0} {Y0 - 1} {Z0} {X0 + 2} {Y0 - 1} {Z0} minecraft:lava")
        both_(f"setblock {X0 + 4} {Y0} {Z0} minecraft:cactus")
        both_(f"setblock {X0 + 6} {Y0 - 1} {Z0} minecraft:magma_block")
        both_(item("f0", X0 + 0.5, Y0 + 5.0, Z0 + 0.5))
        both_(item("f1", X0 + 4.5, Y0 + 0.0, Z0 + 0.5, mot=(0.0, 0.0, 0.0)))
        both_(item("f2", X0 + 6.5, Y0 + 0.5, Z0 + 0.5))
        return ["f0", "f1", "f2"], 60

    def piston_push():
        clear()
        both_(f"setblock {X0} {Y0} {Z0} minecraft:piston[facing=east]")
        both_(f"setblock {X0 + 3} {Y0} {Z0} minecraft:stone")
        both_(item("p0", X0 + 1.5, Y0 + 0.0, Z0 + 0.5))
        both_(item("p1", X0 + 1.5, Y0 + 1.0, Z0 + 0.5))
        both_(f"setblock {X0 - 1} {Y0} {Z0} minecraft:redstone_block")
        return ["p0", "p1"], 40

    def minecart_hopper():
        clear()
        both_(f"fill {X0} {Y0} {Z0} {X0 + 10} {Y0} {Z0} minecraft:powered_rail[shape=east_west,powered=true]")
        both_(f"fill {X0 + 11} {Y0} {Z0} {X0 + 12} {Y0} {Z0} minecraft:rail[shape=east_west]")
        both_(f"fill {X0} {Y0 - 1} {Z0} {X0 + 12} {Y0 - 1} {Z0} minecraft:redstone_block")
        both_(f'summon minecraft:hopper_minecart {X0 + 0.5} {Y0 + 0.1} {Z0 + 0.5} {{Tags:["m0"],Motion:[0.0d,0.0d,0.0d]}}')
        both_(item("m1", X0 + 3.5, Y0 + 0.3, Z0 + 0.5, pickup=0))
        both_(item("m2", X0 + 6.5, Y0 + 0.3, Z0 + 0.5, pickup=0))
        return ["m0"], 90

    S["water_channel"] = water_channel
    S["bubble_up"] = bubble_up
    S["bubble_down"] = bubble_down
    S["ground_slide"] = ground_slide
    S["fall_lava_cactus_magma"] = fall_and_lava
    S["piston_push"] = piston_push
    S["minecart_hopper"] = minecart_hopper
    return S


def hopper_chain():
    """Items dropped into a hopper that feeds a chain (hopper -> hopper -> chest, a dropper loop): contents of the chests every 20 ticks."""
    clear()
    both_(f"setblock {X0} {Y0} {Z0} minecraft:hopper[facing=east]")
    both_(f"setblock {X0 + 1} {Y0} {Z0} minecraft:hopper[facing=east]")
    both_(f"setblock {X0 + 2} {Y0} {Z0} minecraft:hopper[facing=down]")
    both_(f"setblock {X0 + 2} {Y0 - 1} {Z0} minecraft:chest{{}}") if False else None
    both_(f"fill {X0 + 2} {Y0 - 1} {Z0} {X0 + 2} {Y0 - 1} {Z0} minecraft:chest")
    for i in range(24):
        both_(item(f"h{i}", X0 + 0.5, Y0 + 1.0 + 0.2 * (i % 3), Z0 + 0.5, pickup=0, stack="minecraft:cobblestone" if i % 2 else "minecraft:oak_log"))
    rows = []
    bad = None
    for k in range(0, 21):
        step(10)
        a = ask("K", f"data get block {X0 + 2} {Y0 - 1} {Z0} Items", wait=0.25)
        b = ask("V", f"data get block {X0 + 2} {Y0 - 1} {Z0} Items", wait=0.25)
        na = re.findall(r"count: (\d+)", a)
        nb = re.findall(r"count: (\d+)", b)
        ia = sum(map(int, na))
        ib = sum(map(int, nb))
        rows.append((10 * (k + 1), ia, ib))
        if ia != ib and bad is None:
            bad = (10 * (k + 1), ia, ib)
    print(("SAME  " if bad is None else "DIFF  ") + "hopper_chain", rows[:12], flush=True)


def main():
    setup()
    both_("gamemode creative KilnTest")
    both_("tp KilnTest 28 205 14")
    both_("effect give KilnTest minecraft:slow_falling 99999 0 true")
    want = sys.argv[1:]
    S = scenarios()
    for name, fn in S.items():
        if want and name not in want:
            continue
        tags, ticks = fn()
        watch(name, tags, ticks)
    if not want or "hopper_chain" in want:
        hopper_chain()
    both_("tick unfreeze")


main()
