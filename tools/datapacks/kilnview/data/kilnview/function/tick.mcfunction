bossbar set kilnview:ticks players @a
scoreboard players add #t viewticks 1
execute if score #t viewticks matches 101.. run scoreboard players set #t viewticks 0
execute store result bossbar kilnview:ticks value run scoreboard players get #t viewticks
title @a actionbar [{"text":"tick function: ","color":"gold"},{"score":{"name":"#t","objective":"viewticks"},"color":"yellow"}]
