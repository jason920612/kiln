scoreboard players add #ticks kw84 1
execute as @a[predicate=kw84:holding_gold] at @s run particle minecraft:happy_villager ~ ~1 ~ 0.3 0.3 0.3 0 2
execute as @a[predicate=kw84:holding_gold] run scoreboard players add @s kw84 1
execute if score #ticks kw84 matches 100.. run function kw84:cycle
