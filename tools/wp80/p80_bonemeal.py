"""wp80 items 4/7: bone meal from dispensers on many plants (40 tiles each): the number of non-air blocks around the target afterwards, Kiln against vanilla (statistics)."""
from lab80 import *
import statistics
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false"))
both_("difficulty normal"); both_("time set 6000")
Y = 150
CASES = {
 "pale_moss_block": ("minecraft:pale_moss_block", "stone", None),
 "moss_block": ("minecraft:moss_block", "stone", None),
 "grass_block": ("minecraft:grass_block", "stone", None),
 "torchflower_crop": ("minecraft:torchflower_crop[age=0]", "farmland", None),
 "pitcher_crop": ("minecraft:pitcher_crop[age=0,half=lower]", "farmland", None),
 "wheat": ("minecraft:wheat[age=0]", "farmland", None),
 "sweet_berry": ("minecraft:sweet_berry_bush[age=0]", "grass_block", None),
 "short_grass": ("minecraft:short_grass", "grass_block", None),
 "tall_grass_flowers": ("minecraft:azalea", "grass_block", None),
 "bamboo": ("minecraft:bamboo", "grass_block", None),
 "cactus_no": ("minecraft:sugar_cane", "sand", None),
 "mangrove_propagule": ("minecraft:mangrove_propagule[stage=0,age=0]", "mud", None),
 "oak_sapling": ("minecraft:oak_sapling", "grass_block", None),
 "pale_oak_sapling": ("minecraft:pale_oak_sapling", "grass_block", None),
 "red_mushroom": ("minecraft:brown_mushroom", "mycelium", None),
 "kelp": ("minecraft:kelp[age=0]", "sand", "water"),
 "seagrass": ("minecraft:seagrass", "sand", "water"),
 "glow_lichen": ("minecraft:glow_lichen[down=true]", "stone", None),
 "big_dripleaf": ("minecraft:big_dripleaf[facing=north,tilt=none,waterlogged=false]", "clay", None),
 "small_dripleaf": ("minecraft:small_dripleaf[half=lower,facing=north]", "clay", None),
 "pink_petals": ("minecraft:pink_petals[flower_amount=1,facing=north]", "grass_block", None),
 "wildflowers": ("minecraft:wildflowers[flower_amount=1,facing=north]", "grass_block", None),
 "leaf_litter": ("minecraft:leaf_litter[segment_amount=1,facing=north]", "grass_block", None),
 "bush": ("minecraft:bush", "grass_block", None),
 "firefly_bush": ("minecraft:firefly_bush", "grass_block", None),
 "pale_moss_carpet": ("minecraft:pale_moss_carpet[bottom=true,north=none,east=none,south=none,west=none]", "pale_moss_block", None),
 "flowering_azalea": ("minecraft:flowering_azalea", "grass_block", None),
 "tall_grass": ("minecraft:tall_grass[half=lower]", "grass_block", None),
 "rose_bush": ("minecraft:rose_bush[half=lower]", "grass_block", None),
 "sea_pickle": ("minecraft:sea_pickle[pickles=1,waterlogged=true]", "sand", "water"),
 "melon_stem": ("minecraft:melon_stem[age=0]", "farmland", None),
 "cocoa": ("minecraft:oak_log", "grass_block", None),
 "beetroots": ("minecraft:beetroots[age=0]", "farmland", None),
 "nether_wart_no": ("minecraft:dead_bush", "sand", None),
 "dandelion": ("minecraft:dandelion", "grass_block", None),
 "lily_of_valley": ("minecraft:lily_of_the_valley", "grass_block", None),
 "fern": ("minecraft:fern", "grass_block", None),
 "large_fern": ("minecraft:large_fern[half=lower]", "grass_block", None),
 "bamboo_sapling": ("minecraft:bamboo_sapling", "grass_block", None),
}
names = sys.argv[1].split(",") if len(sys.argv) > 1 else list(CASES)
N = int(sys.argv[2]) if len(sys.argv) > 2 else 36
def tile(i):
    return 4 + 8 * (i % 6), 4 + 8 * (i // 6)
for name in names:
    blk, floor, extra = CASES[name]
    both_("tick unfreeze")
    for i in range(N):
        x, z = tile(i)
        OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 6} {Y + 12} {z + 6} minecraft:air")
        OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 6} {Y - 1} {z + 6} minecraft:{floor}")
        if floor == "farmland":
            OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 6} {Y - 1} {z + 6} minecraft:farmland[moisture=7]")
        if extra == "water":
            OB(f"fill {x - 1} {Y} {z - 1} {x + 6} {Y + 6} {z + 6} minecraft:water")
        OB(f"setblock {x + 1} {Y} {z + 2} {blk}")
        if "pitcher" in name:
            OB(f"setblock {x + 1} {Y + 1} {z + 2} minecraft:pitcher_crop[age=0,half=upper]")
        OB(f"setblock {x} {Y} {z + 2} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:64}}]}}")
    time.sleep(0.5)
    for i in range(N):
        x, z = tile(i)
        OB(f"setblock {x} {Y + 1} {z + 2} minecraft:redstone_block")
    time.sleep(3)
    res = {}
    for t in "KV":
        v = []
        for i in range(N):
            x, z = tile(i)
            vol = 8 * 8 * 13
            air = count_blocks(t, x, Y, z, x + 5, Y + 11, z + 5, "minecraft:air")
            v.append(vol - air - 0)
        res[t] = v
    f = lambda v: f"mean {statistics.mean(v):.2f} sd {statistics.pstdev(v):.2f}"
    print(f"{name:20s} K {f(res['K'])}   V {f(res['V'])}   (n={N})", flush=True)
for i in range(N):
    x, z = tile(i)
    OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 6} {Y + 12} {z + 6} minecraft:air")
