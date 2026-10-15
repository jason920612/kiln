tag @s add wp66seen
execute store result score #t wp66 run time query gametime
execute store result score #x wp66 run data get entity @s Pos[0] 1000
execute store result score #y wp66 run data get entity @s Pos[1] 1000
execute store result score #z wp66 run data get entity @s Pos[2] 1000
data modify storage wp66:log golems append value {}
execute store result storage wp66:log golems[-1].t int 1 run scoreboard players get #t wp66
execute store result storage wp66:log golems[-1].x int 1 run scoreboard players get #x wp66
execute store result storage wp66:log golems[-1].y int 1 run scoreboard players get #y wp66
execute store result storage wp66:log golems[-1].z int 1 run scoreboard players get #z wp66
scoreboard players add #n wp66 1
kill @s
