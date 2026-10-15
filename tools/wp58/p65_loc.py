from p65 import *
import sys
out = []
for cmd in ["execute in minecraft:the_nether positioned 100 64 100 run locate structure minecraft:fortress",
            "execute in minecraft:the_nether positioned 100 64 100 run locate structure minecraft:bastion_remnant",
            "execute in minecraft:the_nether positioned 100 64 100 run locate biome minecraft:basalt_deltas",
            "execute in minecraft:the_nether positioned 100 64 100 run locate biome minecraft:soul_sand_valley",
            "execute in minecraft:the_nether positioned 100 64 100 run locate biome minecraft:warped_forest",
            "execute in minecraft:the_nether positioned 100 64 100 run locate biome minecraft:crimson_forest",
            "execute in minecraft:the_end positioned 0 64 0 run locate structure minecraft:end_city",
            "execute in minecraft:overworld positioned 0 64 0 run locate structure minecraft:village_plains",
            "execute in minecraft:overworld positioned 0 64 0 run locate structure minecraft:woodland_mansion",
            "execute in minecraft:overworld positioned 0 64 0 run locate structure minecraft:pillager_outpost",
            "execute in minecraft:overworld positioned 0 64 0 run locate structure minecraft:stronghold",
            "execute in minecraft:overworld positioned 0 64 0 run locate structure minecraft:ocean_monument"]:
    r = sv(cmd, wait=12.0, show=False)
    l = [x for x in r.splitlines() if "locate" in x or "nearest" in x or "Could not" in x or "chat" in x]
    print(cmd.split("run ")[1], "=>", (l[-1] if l else r[-200:])[:200], flush=True)
