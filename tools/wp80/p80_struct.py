"""wp80 items 2/3: locate structures on both servers (same seed) -> same coordinates?, then census of the entities each one spawns once its chunks are loaded.
usage: p80_struct.py structure_id[,..] [radius]"""
from lab80 import *
ids = sys.argv[1].split(",")
R = int(sys.argv[2]) if len(sys.argv) > 2 else 80
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
revive()
for sid in ids:
    pos = {t: locate_xz(t, f"minecraft:{sid}", 0, 0, wait=25) for t in "KV"}
    print(sid, "located", pos, "SAME" if pos["K"] == pos["V"] else "DIFF", flush=True)
    if not pos["K"] or pos["K"] != pos["V"]:
        continue
    x, z = pos["K"]
    for t in "KV":
        w(t, f"gamemode creative KilnTest")
        w(t, f"execute in minecraft:overworld run forceload add {x - 48} {z - 48} {x + 48} {z + 48}")
        w(t, f"execute in minecraft:overworld run tp KilnTest {x} 120 {z}")
    time.sleep(40)
    cen = {}
    for t in "KV":
        cen[t] = {}
        for ty in ("pillager", "vindicator", "evoker", "vex", "allay", "illusioner", "ravager", "witch", "villager", "iron_golem", "cat", "parrot", "guardian", "elder_guardian", "shulker", "zombie", "skeleton", "spider", "cave_spider", "silverfish", "creeper", "zombie_villager", "item_frame", "glow_item_frame", "armor_stand", "minecart", "chest_minecart", "tnt_minecart", "item", "pig", "chicken"):
            n = count(t, f"@e[type=minecraft:{ty},x={x - 80},y=-64,z={z - 80},dx=160,dy=400,dz=160]")
            if n:
                cen[t][ty] = n
    print(sid, "K", cen["K"], flush=True)
    print(sid, "V", cen["V"], flush=True)
    for t in "KV":
        w(t, f"execute in minecraft:overworld run forceload remove {x - 48} {z - 48} {x + 48} {z + 48}")
