"""wp80 item 8: beacons, lock step with both real players standing near (console only): the pyramid levels (1..4, base blocks of different metals, a mixed base, a glass
block in the way), the effects given (primary / secondary / the same effect twice), their amplifier and duration, the range, the beacon block entity's data."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "natural_regeneration false"))
both_("difficulty normal")
revive()
for t in "KV":
    w(t, "gamemode survival KilnTest")
X, Y, Z = 8, 150, 30


def build(levels, base="minecraft:iron_block", hole=None, glass=False, mixed=False):
    OB(f"fill {X - 12} {Y - 6} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
    for lv in range(1, levels + 1):
        r = lv
        blk = base
        if mixed and lv == 2:
            blk = "minecraft:gold_block"
        OB(f"fill {X - r} {Y - lv} {Z - r} {X + r} {Y - lv} {Z + r} {blk}")
    if hole:
        OB(f"setblock {X + hole[0]} {Y - hole[1]} {Z + hole[2]} minecraft:stone")
    OB(f"setblock {X} {Y} {Z} minecraft:beacon")
    if glass:
        OB(f"setblock {X} {Y + 3} {Z} minecraft:stone")


def probe(tag, effects="speed", sec=None, wait=100, dist=6):
    both_("effect clear KilnTest")
    sec_txt = f",Secondary:\"minecraft:{sec}\"" if sec else ""
    OB(f"data merge block {X} {Y} {Z} {{Primary:\"minecraft:{effects}\"{sec_txt}}}")
    OB(f"tp KilnTest {X + dist}.5 {Y + 1} {Z}.5")
    both_("tick unfreeze")
    time.sleep(wait / 20.0 + 3.0)
    r = {}
    for t in "KV":
        e = ask(t, "data get entity KilnTest active_effects", wait=0.4)
        effs = re.findall(r"\{[^{}]*\}", e)
        d = []
        for x in effs:
            i = re.search(r'id: "minecraft:(\w+)"', x)
            amp = re.search(r"amplifier: (\d+)b", x)
            dur = re.search(r"duration: (\d+)", x)
            if i:
                d.append((i.group(1), int(amp.group(1)) if amp else None, int(dur.group(1)) if dur else None))
        b = nbt(t, "block", f"{X} {Y} {Z}")
        lv = re.search(r"Levels: (\d+)", b)
        r[t] = (sorted((n, a_) for n, a_, _ in d), lv.group(1) if lv else None, [dd for n, a_, dd in d])
    cmpn(f"{tag}: effects (id,amp) / Levels", r["K"][:2], r["V"][:2])
    print(f"      durations K {r['K'][2]} V {r['V'][2]}", flush=True)


for lv in (1, 2, 3, 4):
    build(lv)
    time.sleep(1.0)
    probe(f"pyramid {lv}", "speed", "regeneration" if lv >= 4 else None)
build(4, mixed=True)
probe("pyramid 4 mixed iron/gold", "haste", "regeneration")
build(4, hole=(2, 3, 0))
probe("pyramid 4 with a stone in the 4th layer", "resistance", "regeneration")
build(3, glass=True)
probe("pyramid 3 with stone above the beacon", "speed")
build(4)
probe("pyramid 4 range 41 away", "speed", "speed", dist=40)
probe("pyramid 4 range 30 away", "strength", "regeneration", dist=30)
probe("pyramid 4 range 52 away", "jump_boost", "regeneration", dist=52)
for base in ("minecraft:gold_block", "minecraft:diamond_block", "minecraft:emerald_block", "minecraft:netherite_block"):
    build(2, base=base)
    probe(f"pyramid 2 of {base.split(':')[1]}", "speed")
OB(f"fill {X - 12} {Y - 6} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
