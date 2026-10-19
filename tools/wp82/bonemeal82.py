"""wp82 item 2: bone meal from dispensers on the plants wp80 did not cover (and the huge mushrooms), 40 tiles each, Kiln against vanilla (statistics).
Per tile one plant and a dispenser with ONE bone meal; afterwards the blocks of the tile are counted by kind. usage: bonemeal82.py [names] [N]"""
import statistics
from lab import *
setup(frozen=False, rules=("do_daylight_cycle false",), size=80)
both_("difficulty normal"); both_("time set 6000")
Y = 150
S = 12  # tile spacing
# name: (plant block, floor, the block kinds to count afterwards)
CASES = {
 "glow_lichen": ("minecraft:glow_lichen[down=true]", "stone", ["minecraft:glow_lichen"]),
 "glow_lichen_wall": ("minecraft:glow_lichen[north=true]", "stone", ["minecraft:glow_lichen"]),
 "shelf_mushroom": ("minecraft:shelf_mushroom[facing=north,age=0]", "stone", ["minecraft:shelf_mushroom[age=0]", "minecraft:shelf_mushroom[age=1]"]),
 "carrots": ("minecraft:carrots[age=0]", "farmland", [f"minecraft:carrots[age={a}]" for a in range(8)]),
 "potatoes": ("minecraft:potatoes[age=0]", "farmland", [f"minecraft:potatoes[age={a}]" for a in range(8)]),
 "wheat": ("minecraft:wheat[age=0]", "farmland", [f"minecraft:wheat[age={a}]" for a in range(8)]),
 "brown_mushroom": ("minecraft:brown_mushroom", "mycelium", ["minecraft:brown_mushroom", "minecraft:brown_mushroom_block", "minecraft:mushroom_stem"]),
 "red_mushroom": ("minecraft:red_mushroom", "mycelium", ["minecraft:red_mushroom", "minecraft:red_mushroom_block", "minecraft:mushroom_stem"]),
 "sculk_vein": ("minecraft:sculk_vein[down=true]", "stone", ["minecraft:sculk_vein"]),
}
names = sys.argv[1].split(",") if len(sys.argv) > 1 and sys.argv[1] != "all" else list(CASES)
N = int(sys.argv[2]) if len(sys.argv) > 2 else 40
def tile(i):
    return 4 + S * (i % 5), 4 + S * (i // 5)
for name in names:
    blk, floor, kinds = CASES[name]
    for i in range(N):
        x, z = tile(i)
        OB(f"fill {x - 3} {Y - 1} {z - 3} {x + 8} {Y + 12} {z + 8} minecraft:air")
        OB(f"fill {x - 3} {Y - 1} {z - 3} {x + 8} {Y - 1} {z + 8} minecraft:{floor}")
        if floor == "farmland":
            OB(f"fill {x - 3} {Y - 1} {z - 3} {x + 8} {Y - 1} {z + 8} minecraft:farmland[moisture=7]")
        if "wall" in name or "shelf" in name:
            OB(f"setblock {x + 1} {Y} {z + 3} minecraft:stone")
        OB(f"setblock {x + 1} {Y} {z + 2} {blk}")
        OB(f"setblock {x} {Y} {z + 2} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:1}}]}}")
    time.sleep(0.5)
    for i in range(N):
        x, z = tile(i)
        OB(f"setblock {x} {Y + 1} {z + 2} minecraft:redstone_block")
    time.sleep(3.5)
    res = {}
    for t in "KV":
        per = {k: [] for k in kinds}
        # one count per row of five tiles (a fill takes 32768 blocks at most)
        for row in range((N + 4) // 5):
            z = 4 + S * row
            for k in kinds:
                per[k].append(count_blocks(t, 2, Y, z - 2, 4 + S * 4 + 7, Y + 11, z + 7, k))
        res[t] = per
    F(f"== {name} (n={N})")
    for k in kinds:
        a, b = res["K"][k], res["V"][k]
        f = lambda v: f"total {sum(v):4d} per row {v}"
        F(f"   {k:40s} K {f(a)}   V {f(b)}")
for i in range(N):
    x, z = tile(i)
    OB(f"fill {x - 3} {Y - 1} {z - 3} {x + 8} {Y + 12} {z + 8} minecraft:air")
