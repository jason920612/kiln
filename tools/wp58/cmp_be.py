"""wp66 item 5: every block entity type, placed with /setblock, `data get block`, on both servers: the saved forms side by side.
usage: TARGET=K|V cmp_be.py OUT.json"""
import sys, json, re, time
from p66rt import *
import snbt

BLOCKS = """chest trapped_chest ender_chest barrel furnace blast_furnace smoker hopper dropper dispenser brewing_stand
jukebox enchanting_table lectern bell campfire soul_campfire beacon comparator daylight_detector end_portal end_gateway
mob_spawner trial_spawner vault oak_sign oak_wall_sign oak_hanging_sign white_bed red_banner white_wall_banner player_head skeleton_skull
piston_head conduit sculk_sensor calibrated_sculk_sensor sculk_shrieker sculk_catalyst decorated_pot chiseled_bookshelf crafter
suspicious_sand suspicious_gravel command_block chain_command_block repeating_command_block structure_block jigsaw
creaking_heart white_shulker_box shulker_box bee_nest beehive flower_pot potted_oak_sapling cherry_shelf end_portal_frame sculk
test_block copper_chest copper_golem_statue""".split()
base_x, base_z, y = 1100, 1100, -50
run([f"forceload add {base_x} {base_z} {base_x+40} {base_z+40}"])
assert ready(base_x, base_z)
res = {}
for n, b in enumerate(BLOCKS):
    x, z = base_x + 2 * (n % 20), base_z + 2 * (n // 20)
    props = {"piston_head": "[type=normal,facing=up]", "oak_wall_sign": "[facing=north]", "white_wall_banner": "[facing=north]"}.get(b, "")
    out = run([f"setblock {x} {y} {z} minecraft:{b}{props}", f"data get block {x} {y} {z}"])
    line = next((l for l in out if "{" in l and ("block data" in l or "commands.data.block.query" in l)), None)
    try:
        nbt = snbt.nbt_of(line) if line else None
    except Exception as e:
        nbt = {"PARSE_ERROR": str(e)}
    res[b] = {"set": out[0][:80], "nbt": nbt, "raw": line[-400:] if line else out[-1][:120]}
json.dump(res, open(sys.argv[1], "w"), indent=1)
print("done", len(res))
