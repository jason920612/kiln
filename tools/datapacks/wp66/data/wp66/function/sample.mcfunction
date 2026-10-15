execute store result score #vx wp66 run data get entity @s Pos[0] 1
scoreboard players operation #vx wp66 -= #x0 wp66
execute if score #vx wp66 matches -99..-1 run scoreboard players add #bm wp66 1
execute if score #vx wp66 matches 0 run scoreboard players add #b0 wp66 1
execute if score #vx wp66 matches 1 run scoreboard players add #b1 wp66 1
execute if score #vx wp66 matches 2 run scoreboard players add #b2 wp66 1
execute if score #vx wp66 matches 3 run scoreboard players add #b3 wp66 1
execute if score #vx wp66 matches 4 run scoreboard players add #b4 wp66 1
execute if score #vx wp66 matches 5 run scoreboard players add #b5 wp66 1
execute if score #vx wp66 matches 6 run scoreboard players add #b6 wp66 1
execute if score #vx wp66 matches 7 run scoreboard players add #b7 wp66 1
execute if score #vx wp66 matches 8 run scoreboard players add #b8 wp66 1
execute if score #vx wp66 matches 9..99 run scoreboard players add #b9 wp66 1
execute if entity @e[type=minecraft:zombie,distance=..8] run scoreboard players add #near wp66 1
scoreboard players add #samples wp66 1
