from lock import *
for what in ("minecraft:jungle_pyramid", "minecraft:swamp_hut", "minecraft:igloo", "minecraft:pillager_outpost", "minecraft:woodland_mansion"):
    r = ask_both(f"execute in minecraft:overworld positioned 0 64 0 run locate structure {what}", wait=10)
    print(what, {t: r[t].strip().replace(chr(10)," ")[-100:] for t in "KV"})
