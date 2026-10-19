scoreboard players set #ticks kw84 0
scoreboard players add #cycles kw84 1
execute as @a at @s if predicate kw84:in_rain run effect give @s minecraft:speed 3 0 true
execute as @a at @s if predicate kw84:night run title @s actionbar {"text":"kw84 night cycle"}
