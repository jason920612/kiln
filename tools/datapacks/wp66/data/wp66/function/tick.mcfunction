execute store result score #lastg wp66 run time query gametime
scoreboard players operation #m wp66 = #lastg wp66
scoreboard players operation #m wp66 %= #20 wp66
execute if score #m wp66 matches 0 if score #on wp66 matches 1 as @e[type=minecraft:villager,tag=wp66] at @s run function wp66:sample
execute as @e[type=minecraft:iron_golem,tag=!wp66seen] at @s run function wp66:golem
