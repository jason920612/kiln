$give $(who) $(item) $(amount)
$tellraw $(who) {"text":"macro gave $(amount) x $(item)","color":"green"}
$scoreboard players set #macro kw84 $(amount)
