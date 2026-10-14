from p65 import *
print(sv("gamerule spawn_mobs", show=False))
print(sv("execute in minecraft:the_nether run summon minecraft:blaze 277 80 234", show=False))
print(sv("kill @e[type=minecraft:blaze]", show=False, wait=0.6))
print(sv("data get entity @e[type=minecraft:player,limit=1] Pos", show=False))
print(sv("execute as @a at @s run say here", show=False))
