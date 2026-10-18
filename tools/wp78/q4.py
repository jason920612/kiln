from lock import *
for c in ["execute in minecraft:the_end if entity @e[type=ender_dragon]", "execute in minecraft:the_end if entity @e[type=experience_orb]", "execute in minecraft:the_end if entity @e[type=end_crystal]", "execute in minecraft:the_end run data get entity @e[type=ender_dragon,limit=1] Health", "execute in minecraft:the_end run data get entity @e[type=ender_dragon,limit=1] DragonPhase"]:
    r = ask("V", c, wait=1.0)
    print(c, r.strip()[-90:])
