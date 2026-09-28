"""Differential command test: run the same console command lines on the vanilla 26.3 server
and on Kiln, and compare the console feedback line by line.

usage: python tools/command_diff.py [--kiln-port 25587] [--vanilla-port 25591] [-k TEXT] [-v]

Needs KILN_WORK (default <repo>/work) with the vanilla server.jar and versions/26.3/ (scratch
files go to KILN_DIFF_SCRATCH, default $KILN_WORK/wp2-commands/diff), and
built executables (cargo build -p kiln-server -p kiln-bot). Both servers get a flat world and
two idle kiln-bots ("Diff0", then "Other0") for selectors. Kiln prints English on its
console through KILN_LANG (en_us.json extracted from the vanilla jar into the scratch dir).

Case syntax (CASES below): one command per line; "# ..." starts a section; "! cmd" runs on
both servers without comparing; "!v cmd" runs on vanilla only (setup Kiln lacks).
"""

import argparse
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK", ROOT / "work"))
SCRATCH = Path(os.environ.get("KILN_DIFF_SCRATCH", WORK / "wp2-commands" / "diff"))
VERSION = "26.3"
DATAPACK = ROOT / "tools" / "datapacks" / "kilndiff"
UNKNOWN = "Unknown or incomplete command. See below for error"

# Test area: chunks -1..1 around 0,0, y 100..170, cleared to air first. Vanilla's flat world
# spawns animals as chunks generate and drops items for `destroy`; they are killed before the
# sections that select entities (Kiln has neither). Periodic animal spawns are turned off.
CASES = r"""
! gamerule spawn_mobs false
!v forceload add -32 -32 47 47
! kill @e[type=!minecraft:player]
! fill -16 100 -16 31 113 31 air
! fill -16 114 -16 31 127 31 air
! fill -16 128 -16 31 141 31 air
! fill -16 142 -16 31 155 31 air
! fill -16 156 -16 31 169 31 air
! tp Diff0 8 160 8 0 0
! tp Other0 4 160 12 90 0

# loaded area
execute if loaded -16 100 -16
execute if loaded 31 100 31
execute if loaded 100000 100 0

# setblock
setblock 0 100 0 minecraft:stone
setblock 0 100 0 stone
execute if block 0 100 0 stone
setblock 0 100 0 dirt keep
setblock 1 100 0 dirt keep
execute if block 1 100 0 minecraft:dirt
setblock 0 100 0 oak_log[axis=x] replace
execute if block 0 100 0 oak_log[axis=x]
execute if block 0 100 0 oak_log[axis=y]
execute if block 0 100 0 #minecraft:logs
execute if block 0 100 0 #minecraft:logs[axis=z]
execute unless block 0 100 0 #minecraft:logs
setblock 0 100 0 air destroy
execute if block 0 100 0 air
setblock 0 100 0 stone strict
setblock 0 100 0 oak_stairs[facing=east,half=top]
execute if block 0 100 0 oak_stairs[half=top]
execute if block 0 100 0 oak_stairs[facing=west]
execute if block 0 100 0 minecraft:oak_stairs[facing=east,waterlogged=false]
setblock 0 100 0 chest{Items:[]}
setblock 0 100 0 stone{}
setblock 2 100 0 chest[facing=north]{CustomName:"x"}
execute if block 2 100 0 chest{}
setblock 0 400 0 stone
setblock 0 -65 0 stone
setblock 100000 100 0 stone
setblock 0 100 0 minecraft:nonexistent
setblock 0 100 0 oak_log[axis=q]
setblock 0 100 0 oak_log[foo=bar]
setblock 0 100 0 oak_stairs[facing=east,facing=west]
setblock 0 100 0 oak_log[axis=x
setblock 0 100 0 oak_log[axis]
setblock 0 100 0 #minecraft:logs
setblock 0 100 0 chest{Items:[}
setblock 0 100 0 stone replace extra
setblock 0 100
setblock 0 100 0 stone bogus

# fill
fill 0 101 0 3 103 3 stone
fill 0 101 0 3 103 3 stone
fill 3 103 3 0 101 0 glass outline
execute if block 1 102 1 stone
execute if block 0 101 0 glass
fill 0 101 0 3 103 3 air hollow
execute if block 1 102 1 air
fill 0 101 0 3 103 3 dirt keep
fill 0 101 0 3 103 3 dirt keep
fill 0 101 0 3 103 3 stone replace dirt
fill 0 101 0 3 103 3 cobblestone replace #minecraft:base_stone_overworld
fill 0 101 0 3 103 3 stone replace air
fill 0 101 0 3 103 3 stone replace
fill 0 101 0 3 103 3 granite replace minecraft:cobblestone strict
fill 0 101 0 3 103 3 air destroy
fill 0 101 0 3 103 3 stone strict
fill 0 101 0 3 103 3 oak_log[axis=z]
execute if block 3 103 3 oak_log[axis=z]
fill 0 101 0 40 140 40 stone
fill 0 380 0 1 390 1 stone
fill 0 318 0 1 322 1 stone
fill 99990 101 0 100000 101 0 stone
fill 0 101 0 3 103 3 #minecraft:logs
fill 0 101 0 3 103 3 stone replace #minecraft:nonexistent_tag
fill 0 101 0 3 103 3 stone replace stone[foo=bar]
gamerule max_block_modifications 10
fill 0 101 0 3 103 3 air
fill 0 101 0 1 101 1 air
gamerule max_block_modifications 32768
fill 0 101 0 3 103 3 air

# clone
! fill 0 110 0 2 112 2 stone
! setblock 1 111 1 glass
! setblock 0 110 0 air
clone 0 110 0 2 112 2 10 110 0
execute if blocks 0 110 0 2 112 2 10 110 0 all
execute if blocks 0 110 0 2 112 2 10 110 0 masked
execute if blocks 0 110 0 2 112 2 11 110 0 all
clone 0 110 0 2 112 2 10 110 0
clone 0 110 0 2 112 2 20 110 0 masked
execute if block 20 110 0 air
clone 0 110 0 2 112 2 20 115 0 filtered glass
execute if block 21 116 1 glass
execute if block 20 115 0 air
clone 0 110 0 2 112 2 1 110 0
clone 0 110 0 2 112 2 1 110 0 replace force
clone 0 110 0 2 112 2 10 120 0 replace move
execute if block 1 111 1 air
execute if block 11 121 1 glass
clone 10 120 0 12 122 2 0 110 0 masked move
clone -16 100 -16 31 120 31 -16 130 -16
clone 0 100 0 40 140 40 0 100 5000
clone 0 110 0 2 112 2 10 318 0
clone 0 110 0 2 112 2 100000 110 0
clone 100000 110 0 100002 112 2 0 110 0
clone from minecraft:overworld 0 110 0 2 112 2 to minecraft:overworld 25 110 0
clone from minecraft:the_nether 0 110 0 2 112 2 to minecraft:overworld 25 110 0
clone from minecraft:overworld 0 110 0 2 112 2 to minecraft:the_nether 25 110 0
clone 0 110 0 2 112 2 25 110 5 strict
clone 0 110 0 2 112 2 25 110 5 filtered #minecraft:logs
clone 0 110 0 2 112 2 25 110 5 filtered stone force
clone 0 110 0 2 112 2 25 110 5 masked bogus

# execute conditions
! kill @e[type=!minecraft:player]
execute if blocks 0 110 0 2 112 2 25 110 0 all
execute if blocks 0 110 0 2 112 2 25 110 0 masked
execute unless blocks 0 110 0 2 112 2 26 110 0 all
execute if blocks 0 100 0 40 140 40 0 100 0 all
execute if blocks 0 110 0 2 112 2 100000 110 0 all
execute if entity @a
execute if entity Diff0
execute if entity @e[type=minecraft:pig]
execute unless entity @e[type=minecraft:pig]
execute if entity @e[type=minecraft:nonexistent]
execute if dimension minecraft:overworld
execute if dimension minecraft:the_nether
execute if dimension minecraft:nonexistent
execute if loaded 0 100 0
execute unless loaded 0 100 0
execute if biome 0 100 0 minecraft:plains
execute if biome 0 100 0 #minecraft:is_overworld
execute if biome 0 100 0 minecraft:desert
execute if biome 0 100 0 minecraft:nonexistent
execute if biome 100000 100 0 minecraft:plains
execute if block 100000 100 0 stone
execute if function minecraft:nope
execute if function #minecraft:nope
execute if score Diff0 nosuchobjective matches 1
execute if data block 0 110 0 Items
execute if data storage minecraft:nothing foo
execute if block 1 111 1 stone run execute if block 1 111 1 stone

# execute modifiers
execute positioned 5 120 5 run setblock ~ ~ ~ stone
execute positioned 5.7 120.2 5.9 align xz run setblock ~ ~ ~1 dirt
execute if block 5 120 6 dirt
execute positioned 5.7 120.9 5.2 align xyz positioned ~0.5 ~ ~0.5 run setblock ~ ~ ~ glass
execute positioned 5 120 5 rotated 90 0 run setblock ^ ^ ^2 glass
execute if block 3 120 5 glass
execute positioned 5 120 5 rotated -90 0 run setblock ^ ^ ^2 glass
execute positioned 5 120 5 facing 5 120 10 run setblock ^ ^ ^3 glass
execute if block 5 120 8 glass
execute positioned 5 120 5 facing 10 125 5 run setblock ^ ^ ^3 dirt
execute positioned 5 120 5 rotated 0 -90 run setblock ^ ^ ^3 dirt
execute positioned 5 120 5 positioned over world_surface run setblock ~ ~ ~ glass
execute positioned 5 120 5 positioned over motion_blocking run setblock ~ ~ ~ glass
execute positioned 5 120 5 positioned over motion_blocking_no_leaves run setblock ~ ~ ~ glass
execute positioned 5 120 5 positioned over ocean_floor run setblock ~ ~ ~ glass
execute as Diff0 at @s run setblock ~ ~-1 ~ glass
execute as Diff0 at @s anchored eyes run setblock ^ ^ ^1 glass
execute as Diff0 at @s run setblock ^ ^ ^2 glass
execute as Diff0 at @s anchored eyes run setblock ^ ^ ^2 dirt
execute as Diff0 at @s anchored feet run setblock ^ ^ ^3 dirt
execute as Diff0 at @s facing entity @s eyes run setblock ^ ^ ^3 stone
execute as Diff0 at @s facing entity @s feet run setblock ^ ^1 ^1 stone
execute at Diff0 facing 8 160 20 run setblock ^1 ^ ^1 stone
execute positioned as Diff0 run setblock ~ ~2 ~ glass
execute rotated as Diff0 positioned 0 120 0 run setblock ^ ^ ^1 glass
execute in minecraft:the_nether run setblock 0 100 0 stone
execute in minecraft:overworld positioned 1 100 1 run setblock ~ ~ ~ glass
execute in minecraft:nonexistent run setblock 0 100 0 stone
execute run execute run setblock 2 100 2 stone
execute as @a run execute as @s run setblock 3 100 3 stone
execute as @e[type=minecraft:pig] run setblock 4 100 4 stone
execute at @e[type=minecraft:pig] run setblock 4 100 4 stone
execute as @a at @s run setblock ~1 ~ ~ glass
execute summon minecraft:nonexistent run setblock 4 100 4 stone
execute on passengers run setblock 4 100 4 stone
execute as Diff0 on vehicle run setblock 4 100 4 stone
execute positioned 5 120 5 run execute if block ~ ~ ~ stone
execute align xyz
execute align xyzx run say hi
execute rotated 0
execute facing entity @a feet
execute anchored head run say hi
execute run
execute bogus

# execute store
execute store result score Diff0 nosuchobjective run setblock 6 100 6 stone
execute store result block 0 110 0 Items int 1 run setblock 6 100 6 stone
execute store success bossbar minecraft:nosuchbar value run setblock 6 100 6 dirt
execute store result storage minecraft:x foo int 1 run setblock 6 100 7 dirt
execute if data storage minecraft:x foo
execute if data storage minecraft:x bar
execute if data storage minecraft:x {foo:1}
execute store result storage minecraft:x a.b[0] int 1 run setblock 6 100 8 dirt
execute if data storage minecraft:x a.b
execute store result storage minecraft:new a.b[0] int 1 run setblock 6 100 9 dirt
execute if data storage minecraft:new a
execute store success storage minecraft:x l[] byte 1 run setblock 6 100 10 dirt
execute if data storage minecraft:x l[]
execute store result storage minecraft:x f float 0.5 run fill 6 101 6 8 101 6 dirt
execute if data storage minecraft:x {f:1.5f}
execute store result storage minecraft:y x.y.z long 10 run fill 6 102 6 8 102 6 dirt
execute if data storage minecraft:y x.y{z:30L}
execute store result storage minecraft:y s short 1000 run fill 6 103 6 8 103 6 dirt
execute if data storage minecraft:y {s:3000s}
execute store result storage minecraft:y b byte 100 run fill 6 104 6 8 104 6 dirt
execute if data storage minecraft:y {b:44b}
execute store result storage minecraft:y d double -0.1 run fill 6 105 6 8 105 6 dirt
execute if data storage minecraft:y d
execute store result storage minecraft:y l[{id:1}].v int 1 run fill 6 106 6 8 106 6 dirt
execute if data storage minecraft:y l[{id:1}]
execute if data storage minecraft:y l[0].v
execute if data storage minecraft:empty x
execute store result block 0 110 0 Items bogus 1 run say hi

# game rules for execution limits
gamerule max_command_forks 1
execute as @a run say forked
execute run say not forked
gamerule max_command_forks 65536
execute as @a run say forked
gamerule max_command_sequence_length 1
execute as @a run say limited
gamerule max_command_sequence_length 65536

# forks over two players
! kill @e[type=!minecraft:player]
execute as @a run say hi
execute as @a at @s run say at
execute as @a as @a run say nested
execute as @a run execute as @a run say nested2
execute as @a[limit=1] run say one
execute as @e[type=minecraft:player] run say typed
execute as @a[name=!Diff0] run say not0
execute if entity @a[name=!Diff0]
execute as @a if entity @s[name=Other0] run say filtered
execute as @a unless entity @s[name=Other0] run say filtered2
execute as @a at @s if block ~ ~-1 ~ glass run say onglass
execute as @a at @s run setblock ~ ~2 ~ stone
execute as @a at @s rotated as @s run setblock ^ ^ ^2 dirt
execute at @a positioned ~ ~3 ~ run setblock ~ ~ ~ glass
execute as @a[limit=0] run say none
gamerule max_command_forks 3
execute as @a as @a run say limited
execute as @a run execute as @a run say limited2
execute as @a at @s as @a run say limited3
gamerule max_command_forks 4
execute as @a as @a run say limited4
gamerule max_command_forks 2
execute as @a run say limited5
gamerule max_command_forks 65536
gamerule max_command_sequence_length 3
execute as @a run say seq
execute as @a as @a run say seq2
execute run execute run say seq3
gamerule max_command_sequence_length 5
execute as @a as @a run say seq4
gamerule max_command_sequence_length 65536

# tellraw
tellraw Diff0 "hello"
tellraw Diff0 {"text":"a","color":"red"}
tellraw Diff0 [{"text":"a"},{"selector":"@a"}]
tellraw Diff0 {"translate":"chat.type.text","with":["a","b"]}
tellraw Diff0 {text:"unquoted",bold:1b}
tellraw Diff0 {"text":"a"
tellraw Diff0 {"text":"a",}
tellraw Diff0 {"score":{"name":"*","objective":"x"}}
tellraw Nobody "x"
tellraw @e[type=minecraft:pig] "x"
tellraw @e "x"
tellraw Diff0
tellraw Diff0 'single'
tellraw Diff0 1
tellraw Diff0 1b
tellraw Diff0 [I;1,2]
tellraw Diff0 []
tellraw Diff0 ["a",1]
tellraw Diff0 [1,2]
tellraw Diff0 {foo:"bar"}
tellraw Diff0 {text:1}
tellraw Diff0 {text:"a",color:"nocolor"}
tellraw Diff0 {text:"a",extra:[]}

# scoreboard
! kill @e[type=!minecraft:player]
scoreboard objectives list
scoreboard players list
scoreboard objectives add kills dummy
scoreboard objectives add kills dummy
scoreboard objectives list
scoreboard objectives add hp health {"text":"Health","color":"red"}
scoreboard objectives add bad nosuchcriterion
scoreboard objectives add t trigger
scoreboard objectives add stat minecraft.mined:minecraft.stone
scoreboard objectives add stat2 minecraft.mined:minecraft.nothing
scoreboard objectives add team teamkill.red
scoreboard objectives remove stat
scoreboard objectives remove team
scoreboard objectives remove nope
scoreboard objectives modify kills displayname {"text":"Kills"}
scoreboard objectives modify kills displayname {"text":"Kills"}
scoreboard objectives modify kills displayname "kills"
scoreboard objectives modify kills rendertype hearts
scoreboard objectives modify kills rendertype hearts
scoreboard objectives modify kills rendertype integer
scoreboard objectives modify kills displayautoupdate true
scoreboard objectives modify kills displayautoupdate true
scoreboard objectives modify kills displayautoupdate false
scoreboard objectives modify kills numberformat styled {color:"red"}
scoreboard objectives modify kills numberformat fixed "x"
scoreboard objectives modify kills numberformat blank
scoreboard objectives modify kills numberformat
scoreboard objectives modify nope rendertype hearts
scoreboard objectives setdisplay sidebar kills
scoreboard objectives setdisplay sidebar kills
scoreboard objectives setdisplay sidebar
scoreboard objectives setdisplay sidebar
scoreboard objectives setdisplay sidebar.team.red kills
scoreboard objectives setdisplay below_name t
scoreboard objectives setdisplay nope kills
scoreboard players set Diff0 kills 5
scoreboard players set #fake kills 0
scoreboard players set @a kills 7
scoreboard players set Diff0 hp 3
scoreboard players set Diff0 nope 3
scoreboard players get Diff0 kills
scoreboard players get #fake kills
scoreboard players get #nobody kills
scoreboard players get * kills
scoreboard players add Diff0 kills 3
scoreboard players add * kills 1
scoreboard players remove #fake kills 10
scoreboard players add Diff0 kills -1
scoreboard players add @e[type=minecraft:pig] kills 1
scoreboard players list Diff0
scoreboard players list #nobody
scoreboard players operation Diff0 kills += #fake kills
scoreboard players operation Diff0 kills /= #zero kills
scoreboard players get #zero kills
scoreboard players operation Diff0 kills %= #fake kills
scoreboard players operation Diff0 kills >< #fake kills
scoreboard players operation Diff0 kills < #fake kills
scoreboard players operation * kills = Diff0 kills
scoreboard players operation Diff0 kills bogus #fake kills
scoreboard players operation Diff0 hp = #fake kills
scoreboard players enable Diff0 kills
scoreboard players enable Diff0 t
scoreboard players enable Diff0 t
scoreboard players display name Diff0 kills {"text":"D"}
scoreboard players display name Diff0 kills
scoreboard players display name * kills "x"
scoreboard players display numberformat * kills blank
scoreboard players display numberformat Diff0 kills
scoreboard players reset #zero kills
scoreboard players reset #fake
scoreboard players reset * t
scoreboard players reset #nobody
scoreboard players list

# execute store and scores
! kill @e[type=!minecraft:player]
execute store result score Diff0 kills run fill 0 130 0 1 130 1 stone
scoreboard players get Diff0 kills
execute store success score Diff0 kills run fill 0 130 0 1 130 1 stone
scoreboard players get Diff0 kills
execute store result score #count kills run execute if entity @a
scoreboard players get #count kills
execute if score Diff0 kills matches 0
execute if score Diff0 kills matches 1..
execute if score Diff0 kills < #count kills
execute if score Diff0 kills = Diff0 kills
execute unless score #nobody kills matches 0
execute if score #nobody kills = Diff0 kills
execute store result score Diff0 kills run scoreboard players get #count kills
scoreboard players get Diff0 kills
execute as @a store result score @s kills run execute if block 0 130 0 stone
scoreboard players get Diff0 kills
execute store result score Diff0 kills as @a run setblock 0 131 0 stone
scoreboard players get Diff0 kills
execute store success score #s kills run setblock 0 131 0 stone
scoreboard players get #s kills
execute store result score #r kills run scoreboard players set * kills 2
scoreboard players get #r kills
execute store result score #t kills run execute store result score #u kills run fill 0 132 0 2 132 0 dirt
scoreboard players get #t kills
scoreboard players get #u kills
execute store result score @e[type=minecraft:pig] kills run say x
execute as @a store result score @s kills run execute if entity @a
scoreboard players get Diff0 kills
scoreboard players get Other0 kills
execute store result score #n kills as @a run say counted
scoreboard players get #n kills
execute store success score #m kills as @a if entity @s[name=Other0]
scoreboard players get #m kills
execute store result score #f kills as @a as @a run say forked
scoreboard players get #f kills
scoreboard players set @a kills 3
scoreboard players add @a kills 1
scoreboard players operation @a kills += @a kills
scoreboard players list
scoreboard objectives remove kills
scoreboard objectives remove hp
scoreboard objectives remove t
scoreboard players list

# scoreboard display
scoreboard objectives add disp dummy
scoreboard players set Diff0 disp 1
scoreboard players display name Diff0 disp {"text":"Named","color":"gold"}
scoreboard players display name Diff0 disp
scoreboard players display name @a disp "x"
scoreboard players display name #fake disp "fake"
scoreboard players display numberformat Diff0 disp fixed "N"
scoreboard players display numberformat @a disp styled {bold:true}
scoreboard players display numberformat Diff0 disp blank
scoreboard players display numberformat Diff0 disp
scoreboard players display numberformat Diff0 nope blank
scoreboard players display numberformat Diff0 disp styled {color:"nocolor"}
scoreboard objectives modify disp numberformat fixed {"text":"F"}
scoreboard objectives modify disp numberformat styled {italic:true}
scoreboard objectives setdisplay sidebar disp
scoreboard objectives setdisplay list disp
scoreboard objectives modify disp displayautoupdate true
scoreboard players add Diff0 disp 1
scoreboard players get Diff0 disp
scoreboard objectives setdisplay sidebar
scoreboard objectives remove disp
scoreboard objectives setdisplay list

# trigger
scoreboard objectives add trig trigger
scoreboard objectives add notrig dummy
trigger trig
execute as Diff0 run trigger trig
scoreboard players enable Diff0 trig
execute as Diff0 run trigger trig
execute as Diff0 run trigger trig
scoreboard players enable Diff0 trig
execute as Diff0 run trigger trig add 5
execute as Diff0 run trigger trig add 5
scoreboard players enable @a trig
execute as Diff0 run trigger trig set -3
scoreboard players get Diff0 trig
execute as Other0 run trigger trig set 7
scoreboard players get Other0 trig
execute as Diff0 run trigger notrig
execute as Diff0 run trigger nope
execute as Diff0 run trigger trig bogus 1
scoreboard objectives remove trig
scoreboard objectives remove notrig

# teams
team list
team add red
team add red
team add blue {"text":"Blue Team","color":"blue"}
team add green "Green"
team add 1234567890123456789
team add a b
team list
team list red
team list nope
team join red Diff0
team list red
team join red #fake
team join red Other0
team list red
team join red
team join nope Diff0
team join blue @a
team list blue
team list red
team leave Diff0
team leave Diff0
team leave @a
team leave #fake
team join red @a
team join green #a
team join green #b
team join green #c
team join green zz
team list green
team empty red
team empty red
team empty blue
team modify red color red
team modify red color red
team modify red color reset
team modify red color reset
team modify red color nocolor
team modify red color yellow
team modify red displayName {"text":"Reds"}
team modify red displayName {"text":"Reds"}
team modify red friendlyFire false
team modify red friendlyFire false
team modify red friendlyFire true
team modify red friendlyFire true
team modify red seeFriendlyInvisibles false
team modify red seeFriendlyInvisibles false
team modify red seeFriendlyInvisibles true
team modify red nametagVisibility hideForOtherTeams
team modify red nametagVisibility hideForOtherTeams
team modify red nametagVisibility never
team modify red nametagVisibility hideForOwnTeam
team modify red nametagVisibility always
team modify red deathMessageVisibility hideForOwnTeam
team modify red deathMessageVisibility always
team modify red deathMessageVisibility always
team modify red collisionRule pushOwnTeam
team modify red collisionRule pushOwnTeam
team modify red collisionRule pushOtherTeams
team modify red collisionRule never
team modify red collisionRule always
team modify red prefix {"text":"[R] ","color":"red"}
team modify red suffix " *"
team modify red prefix {"text":"[R] ","color":"red"}
team modify nope prefix "x"
team modify red bogus
team join red Diff0
team list red
execute as Diff0 run teammsg hello team
execute as Diff0 run tm hi
execute as Other0 run teammsg nobody
teammsg from the console
scoreboard objectives add tk dummy
scoreboard players set Diff0 tk 1
scoreboard players set @a tk 2
scoreboard players get Diff0 tk
scoreboard objectives remove tk
execute if entity @a[team=red]
execute if entity @a[team=blue]
execute if entity @a[team=!red]
execute if entity @a[team=]
execute as @a[team=red] run say in red
team list
team remove blue
team remove blue
team remove green
team remove 1234567890123456789
team list
team remove red
team list

# titles
title Diff0 title "Hi"
title @a subtitle {"text":"sub"}
title Diff0 actionbar "bar"
title @a actionbar {"selector":"@s"}
title @a times 10 70 20
title Diff0 times 1s 2s 0.5s
title Diff0 times 1d 0 0
title @a clear
title Diff0 reset
title @a reset
title Nobody title "x"
title @e[type=minecraft:pig] title "x"
title Diff0 times -1 2 3
title Diff0 times 1x 2 3
title Diff0 title
title Diff0 bogus

# effects
effect give Diff0 minecraft:speed
effect give Diff0 speed 10 1
effect give Diff0 speed 10 1
effect give Diff0 speed 5 0
effect give Diff0 speed 20 1 true
effect give @a minecraft:haste infinite
effect give @a minecraft:haste infinite
effect give @a haste infinite 2 true
effect give Other0 minecraft:instant_health
effect give Other0 minecraft:saturation 2
effect give Diff0 luck 1000000 255
effect give Diff0 luck 1000001
effect give Diff0 luck 0
effect give Diff0 luck 10 256
effect give Diff0 luck 10 -1
effect give Diff0 luck 10 0 maybe
effect give Diff0 minecraft:not_an_effect
effect give Diff0 speed infinite 1 false extra
effect give Nobody speed
effect give @e[type=minecraft:pig] speed
effect give Diff0
effect clear Diff0 minecraft:jump_boost
effect clear Diff0 minecraft:speed
effect clear Diff0 minecraft:speed
effect clear @a minecraft:haste
effect clear @a
effect clear @a
effect clear
effect clear Nobody
effect clear Diff0 bogus:thing
effect

# bossbar
bossbar list
bossbar add kiln:b "Boss"
bossbar add kiln:b "Boss"
bossbar add minecraft:a {"text":"A","color":"gold"}
bossbar add x:y "XY"
bossbar add Bad:Id "x"
bossbar list
bossbar get kiln:b value
bossbar get kiln:b max
bossbar get kiln:b visible
bossbar get kiln:b players
bossbar get nope:x value
bossbar set kiln:b value 30
bossbar set kiln:b value 30
bossbar set kiln:b value -1
bossbar set kiln:b max 60
bossbar set kiln:b max 60
bossbar set kiln:b max 0
bossbar set kiln:b color red
bossbar set kiln:b color red
bossbar set kiln:b color pink
bossbar set kiln:b style notched_10
bossbar set kiln:b style notched_10
bossbar set kiln:b name "Renamed"
bossbar set kiln:b name "Renamed"
bossbar set kiln:b visible false
bossbar set kiln:b visible false
bossbar get kiln:b visible
bossbar set kiln:b visible true
bossbar set kiln:b visible true
bossbar set kiln:b players Diff0
bossbar set kiln:b players Diff0
bossbar get kiln:b players
bossbar set kiln:b players Other0
bossbar get kiln:b players
bossbar set kiln:b players
bossbar set kiln:b players
bossbar set kiln:b players Nobody
bossbar set kiln:b players @e[type=minecraft:pig]
bossbar set nope:x players Diff0
bossbar set kiln:b bogus
execute store result bossbar kiln:b value run bossbar get kiln:b max
bossbar get kiln:b value
execute store result bossbar kiln:b max run bossbar list
bossbar get kiln:b max
execute store success bossbar minecraft:a value run bossbar get kiln:b max
bossbar get minecraft:a value
bossbar remove kiln:b
bossbar remove kiln:b
bossbar remove x:y
bossbar list
bossbar remove minecraft:a
bossbar list

# functions
datapack list
datapack list enabled
datapack list available
scoreboard players get #loads loaded
execute if score #ticks loaded matches 1..
function kilndiff:setup
function kilndiff:hello
function kilndiff:ret
function kilndiff:retfail
function kilndiff:count
scoreboard players get #n fn
function kilndiff:nested
function kilndiff:retrun
function kilndiff:retrun_cmd
function kilndiff:retrun_none
function kilndiff:recurse
scoreboard players get #r fn
function kilndiff:continued
scoreboard players get #n fn
function kilndiff:feedback
scoreboard players get #f fn
function kilndiff:asall
function kilndiff:bad
function kilndiff:sub/deep
function kilndiff:nope
function nope
function #kilndiff:all
function #kilndiff:rets
function #kilndiff:empty
function #kilndiff:nope
function kilndiff:macro
function kilndiff:macro {msg:"hi",v:5}
scoreboard players get #m fn
function kilndiff:macro {msg:"hi"}
function kilndiff:macro {msg:1.5f,v:2b}
function kilndiff:macro {msg:[1,2],v:1L}
function kilndiff:macro {msg:"a b",v:"x"}
function kilndiff:macroret {v:9}
function kilndiff:hello {x:1}
function kilndiff:hello [1]
function kilndiff:hello {x:
execute store result storage kiln:m v int 1 run scoreboard players get #n fn
function kilndiff:macroret with storage kiln:m
function kilndiff:macro with storage kiln:m
function kilndiff:macro with storage kiln:m nope
function kilndiff:macroret with storage kiln:m v
function kilndiff:macro with block 0 100 0
function kilndiff:macro with block 100000 100 0
execute store result score #x fn run function kilndiff:ret
scoreboard players get #x fn
execute store result score #y fn run function kilndiff:hello
scoreboard players get #y fn
execute store success score #z fn run function kilndiff:retfail
scoreboard players get #z fn
execute store result score #t fn run function #kilndiff:rets
scoreboard players get #t fn
execute if function kilndiff:ret
execute if function kilndiff:ret run say yes
execute if function kilndiff:retfail run say no
execute unless function kilndiff:retfail run say unless
execute if function kilndiff:hello run say never
execute unless function kilndiff:hello run say never2
execute if function #kilndiff:rets run say tagged
execute if function kilndiff:nope run say x
execute if function #kilndiff:nope run say x
execute as @a run function kilndiff:ret
execute as @a run function kilndiff:hello
execute as Diff0 run function kilndiff:asall
return 5
return fail
return run say hi
return run function kilndiff:ret
return run function kilndiff:hello
return run execute if entity @e[type=minecraft:pig]
return
execute store result score #q fn run return 8
scoreboard players get #q fn
execute store success score #q fn run return fail
scoreboard players get #q fn
! schedule function kilndiff:sched 100000t
! schedule function kilndiff:sched 200000t append
schedule clear kilndiff:sched
schedule clear kilndiff:sched
! schedule function #kilndiff:all 100000t
schedule clear #kilndiff:all
schedule function kilndiff:sched 0
schedule function kilndiff:macro 1t
schedule function kilndiff:nope 1t
schedule function kilndiff:sched -1
schedule function kilndiff:sched 1t bogus
! schedule function kilndiff:sched 1t
scoreboard players get #n fn
scoreboard players get #s fn
datapack disable "file/kilndiff"
datapack disable "file/kilndiff"
datapack list
function kilndiff:hello
datapack enable "file/kilndiff"
datapack enable "file/kilndiff"
datapack enable "file/nope"
datapack enable minecart_improvements
datapack disable minecart_improvements
datapack enable file/kilndiff
datapack enable "file/kilndiff" first
datapack disable "file/kilndiff"
datapack enable "file/kilndiff" before "file/nope"
datapack enable "file/kilndiff" before vanilla
datapack disable vanilla
datapack list
datapack enable vanilla first
datapack list
reload
function kilndiff:hello
scoreboard players get #loads loaded
datapack create kilndiff2 "x"
scoreboard objectives remove fn
scoreboard objectives remove loaded

# protocol
transfer example.com
transfer example.com 25566
transfer example.com 25566 Diff0
transfer example.com 25566 @a
transfer "a host" 1 Other0
transfer example.com 0 Diff0
transfer example.com 65536 Diff0
transfer example.com 25566 Nobody
transfer example.com 25566 @e[type=minecraft:pig]
execute as Diff0 run transfer example.com
execute as Diff0 run transfer example.com 25570
dialog show Diff0 minecraft:server_links
dialog show @a minecraft:custom_options
dialog show Diff0 quick_actions
dialog show Diff0 minecraft:nope
dialog show Diff0 {type:"minecraft:notice",title:"Hello"}
dialog show @a {type:"minecraft:confirmation",title:"Sure?",yes:{label:"Yes"},no:{label:"No"}}
dialog show Nobody minecraft:server_links
dialog show Diff0
dialog clear Diff0
dialog clear @a
dialog clear Nobody
dialog bogus

# coordinates, shapes and block entities
setblock 2 100 0 chest[facing=north]{CustomName:"y"}
setblock 2 100 0 chest[facing=north]{CustomName:"y"}
setblock 2 100 0 chest[facing=north]
setblock 0.5 100 0 stone
setblock 0 100 ^ stone
setblock 0 100 stone
execute positioned 0 100 0 run setblock 0 ~0.5 0 stone
setblock 30000000 100 0 stone
setblock 29999999 100 0 stone
setblock -30000001 100 0 stone
fill 0 101 0 0 101 0 stone
fill 0 101 0 2 103 2 glass hollow
execute if block 1 102 1 air
fill 0 101 0 1 102 1 dirt hollow
fill 0 101 0 2 101 2 air outline
fill 0 101 0 2 103 2 stone
clone 0 101 0 2 103 2 0 101 0
clone 0 101 0 2 103 2 0 101 0 replace force
clone 0 101 0 2 103 2 1 101 0 masked move
clone 0 101 0 2 103 2 1 101 0 masked force
clone 0 101 0 2 103 2 1 102 1 replace move
execute if blocks 0 101 0 2 103 2 0 101 0 all
execute if blocks 1 102 1 3 104 3 1 102 1 masked
execute align x positioned 3.7 110.2 4.9 run setblock ~ ~ ~ stone
execute positioned 3.7 110.9 4.9 align y run setblock ~ ~ ~ stone
execute rotated 45 0 positioned 0 115 0 run setblock ^ ^ ^3 dirt
execute rotated 0 45 positioned 0 115 0 run setblock ^ ^ ^3 dirt
execute rotated 0 0 positioned 0 115 0 run setblock ^1 ^ ^ glass
execute rotated 0 0 positioned 0 115 0 run setblock ^ ^1 ^ glass
execute rotated 180 -30 positioned 0.5 115 0.5 run setblock ^ ^ ^4 glass
execute positioned 0 115 0 facing 3 118 3 run setblock ^ ^ ^2 glass
execute positioned 0.5 115 0.5 facing 0.5 125 0.5 run setblock ^ ^ ^2 glass
execute as Diff0 rotated as @s positioned 0 118 0 run setblock ^ ^ ^1 stone
execute as Other0 rotated as @s positioned 0 118 0 run setblock ^ ^ ^1 stone
execute if entity @a[x=8,y=160,z=8,distance=..1]
execute if entity @a[x=0,y=0,z=0,distance=..1]
execute if entity @a[x=8,y=160,z=8,dx=1,dy=1,dz=1]
execute if entity @e[type=minecraft:player,limit=1,sort=furthest]
execute if entity @a[y_rotation=80..100]
execute if entity @a[x_rotation=0]
execute as @a[sort=arbitrary] run say arb
gamerule max_command_forks 0
execute as @a run say zero
execute run say zero2
gamerule max_command_forks 65536
gamerule max_command_sequence_length 0
execute run say quota
execute as @a run say quota2
gamerule max_command_sequence_length 65536
# experience
xp query Diff0 points
xp query Diff0 levels
experience add Diff0 10
experience add Diff0 5 levels
xp query Diff0 levels
xp query Diff0 points
experience set Diff0 3 levels
experience set Diff0 100 points
experience set Diff0 2 points
xp query Diff0 points
experience add @a 7
experience add @a 2 levels
experience add Diff0 -1000 levels
xp query Diff0 levels
experience query Diff0
experience set Diff0 -1
xp set Diff0 0 levels
xp add Other0 30 levels
xp query Other0 levels
xp set Other0 0 levels

# advancements
! advancement revoke @a everything
! recipe take @a *
advancement grant Diff0 only minecraft:story/mine_stone
advancement grant Diff0 only minecraft:story/mine_stone
advancement revoke Diff0 only minecraft:story/mine_stone
advancement revoke Diff0 only minecraft:story/mine_stone
advancement grant Diff0 only minecraft:story/mine_stone get_stone
advancement grant Diff0 only minecraft:story/mine_stone get_stone
advancement revoke Diff0 only minecraft:story/mine_stone get_stone
advancement grant Diff0 only minecraft:story/mine_stone nonsense
advancement grant Diff0 only minecraft:nonexistent
advancement grant Diff0 only nonexistent:thing criterion
advancement grant @a only minecraft:story/root
advancement grant @a only minecraft:story/root
advancement revoke @a only minecraft:story/root
advancement revoke @a only minecraft:story/root
advancement grant Diff0 until minecraft:story/iron_tools
advancement grant Diff0 until minecraft:story/iron_tools
advancement revoke Diff0 through minecraft:story/root
advancement grant Diff0 from minecraft:story/follow_ender_eye
advancement revoke Diff0 from minecraft:story/follow_ender_eye
advancement revoke Diff0 from minecraft:story/follow_ender_eye
advancement grant Diff0 only minecraft:recipes/misc/stick
advancement grant @a only minecraft:adventure/kill_a_mob minecraft:zombie
advancement grant @a only minecraft:adventure/kill_a_mob minecraft:zombie
advancement revoke @a only minecraft:adventure/kill_a_mob minecraft:zombie
advancement revoke Other0 only minecraft:adventure/kill_a_mob minecraft:zombie
advancement grant Diff0 only minecraft:adventure/root
advancement grant Diff0 only minecraft:nether/root
! gamerule show_advancement_messages false
advancement grant Diff0 only minecraft:story/enter_the_nether
advancement grant Diff0 everything
advancement grant Diff0 everything
advancement revoke Diff0 everything
advancement revoke Diff0 everything
advancement grant @a everything
advancement revoke @a everything
! gamerule show_advancement_messages true
advancement grant
advancement grant Diff0
advancement grant Diff0 only
advancement grant Diff0 sideways minecraft:story/root

# recipes
recipe give Diff0 minecraft:stick
recipe give Diff0 minecraft:stick
recipe take Diff0 minecraft:stick
recipe take Diff0 minecraft:stick
recipe give @a minecraft:crafting_table
recipe give @a minecraft:crafting_table
recipe take @a minecraft:crafting_table
recipe give Diff0 minecraft:nonexistent
recipe give Diff0 nonexistent:thing
recipe give Diff0 minecraft:armor_dye
recipe give Diff0 minecraft:repair_item
recipe give Diff0 minecraft:white_banner_duplicate
recipe give Diff0 minecraft:book_cloning
recipe give Diff0 minecraft:decorated_pot
recipe give Diff0 minecraft:firework_rocket
recipe give Diff0 minecraft:firework_star
recipe give Diff0 minecraft:firework_star_fade
recipe give Diff0 minecraft:map_extending
recipe give Diff0 minecraft:shield_decoration
recipe give Diff0 minecraft:brewing/lingering_potion_awkward_blaze_powder
recipe give Diff0 *
recipe give Diff0 *
! recipe take @a *
recipe take @a *
! recipe give Other0 *
! recipe take Other0 *
recipe give

# statistics criteria
scoreboard objectives add st_jump minecraft.custom:minecraft.jump
scoreboard objectives add st_mined minecraft.mined:minecraft.stone
scoreboard objectives add st_deaths deathCount
scoreboard objectives add st_health health
scoreboard objectives add st_bad minecraft.custom:minecraft.nothing
scoreboard objectives add st_bad2 minecraft.used:minecraft.stone_bricks_nope
scoreboard objectives list
scoreboard objectives remove st_jump
scoreboard objectives remove st_mined
scoreboard objectives remove st_deaths
scoreboard objectives remove st_health

# data storage
data get storage kiln:t
data get storage kiln:t a
data merge storage kiln:t {a:1,b:{c:"x",d:2.5d},l:[1,2,3],s:"hello world",f:1.5f,by:3b}
data merge storage kiln:t {a:1}
data get storage kiln:t
data get storage kiln:t a
data get storage kiln:t b
data get storage kiln:t b.c
data get storage kiln:t b.d
data get storage kiln:t b.d 10
data get storage kiln:t b.d -0.5
data get storage kiln:t f 3
data get storage kiln:t by
data get storage kiln:t l
data get storage kiln:t l[]
data get storage kiln:t l[1]
data get storage kiln:t l[-1]
data get storage kiln:t l[5]
data get storage kiln:t s
data get storage kiln:t s 2
data get storage kiln:t nope
data get storage kiln:t b.nope.deep
data get storage kiln:t b.c.deep
data get storage kiln:t {a:1}
data get storage kiln:t {a:2}
data get storage kiln:other
data modify storage kiln:t l append value 4
data modify storage kiln:t l prepend value 0
data modify storage kiln:t l insert 2 value 9
data modify storage kiln:t l insert -1 value 8
data modify storage kiln:t l insert 100 value 8
data get storage kiln:t l
data modify storage kiln:t l[0] set value 7
data modify storage kiln:t l[0] set value 7
data modify storage kiln:t l[] set value 1
data get storage kiln:t l
data modify storage kiln:t b merge value {e:1b}
data modify storage kiln:t b merge value {e:1b}
data modify storage kiln:t a merge value {e:1b}
data modify storage kiln:t b merge value 5
data modify storage kiln:t new.path set value "v"
data get storage kiln:t new
data modify storage kiln:t a append value 1
data modify storage kiln:t s2 set string storage kiln:t s
data modify storage kiln:t s3 set string storage kiln:t s 6
data modify storage kiln:t s4 set string storage kiln:t s 0 5
data modify storage kiln:t s5 set string storage kiln:t s -5
data modify storage kiln:t s6 set string storage kiln:t s 3 1
data modify storage kiln:t s7 set string storage kiln:t a
data modify storage kiln:t s8 set string storage kiln:t f
data modify storage kiln:t s9 set string storage kiln:t b
data modify storage kiln:t s10 set string storage kiln:t
data get storage kiln:t s4
data get storage kiln:t s5
data get storage kiln:t s7
data get storage kiln:t s8
data modify storage kiln:t copy set from storage kiln:t b
data modify storage kiln:t copy2 set from storage kiln:t
data modify storage kiln:t copy set from storage kiln:t nope
data get storage kiln:t copy
data modify storage kiln:t l append from storage kiln:t l[]
data get storage kiln:t l
data modify storage kiln:t merged merge from storage kiln:t b
data get storage kiln:t merged
data remove storage kiln:t l[0]
data remove storage kiln:t l[]
data remove storage kiln:t l[]
data remove storage kiln:t nope
data remove storage kiln:t b.c
data get storage kiln:t b
data modify storage kiln:t x set value [B;1b,2b]
data get storage kiln:t x
data modify storage kiln:t x append value 3
data get storage kiln:t x
data modify storage kiln:t y set value [1L,2L]
data get storage kiln:t y
data modify storage kiln:t z set value 'it"s'
data get storage kiln:t z
data get storage kiln:t z 1
execute store result storage kiln:t n int 1 run data get storage kiln:t y
data get storage kiln:t n
execute if data storage kiln:t n
execute if data storage kiln:t nope
data merge storage kiln:t {}
data get storage kiln:t missing 1
data get

# data blocks
! setblock 4 100 4 chest{Items:[{Slot:0b,id:"minecraft:stone",count:3}]}
data get block 4 100 4 Items
data get block 4 100 4 Items[0].count
data get block 4 100 4 Items[0].count 2.5
data get block 4 100 4 Items[0].id
data get block 4 100 4 Items[0].id 1
data get block 4 100 4 Items[5]
data get block 4 100 4 id
data get block 5 100 4
data get block 100000 100 4
data merge block 4 100 4 {CustomName:"Box"}
data merge block 4 100 4 {CustomName:"Box"}
data get block 4 100 4 CustomName
data remove block 4 100 4 CustomName
data remove block 4 100 4 CustomName
data modify block 4 100 4 Items[0].count set value 5
data get block 4 100 4 Items[0].count
data modify block 4 100 4 Items append value {Slot:1b,id:"minecraft:dirt",count:1}
data get block 4 100 4 Items[1].id
data modify storage kiln:t fromblock set from block 4 100 4 Items[0]
data get storage kiln:t fromblock
execute store result block 4 100 4 Items[0].count byte 1 run data get storage kiln:t a
data get block 4 100 4 Items[0].count
execute if data block 4 100 4 Items[{id:"minecraft:dirt"}]
execute unless data block 4 100 4 Items[{id:"minecraft:dirt"}]

# data entities
data get entity Diff0 XpLevel
data get entity Diff0 foodLevel
data get entity Diff0 Health
data get entity Diff0 Health 2
data get entity Diff0 SelectedItemSlot
data get entity Diff0 Dimension
data get entity Diff0 nope
data merge entity Diff0 {Health:5f}
data modify entity Diff0 Health set value 5f
data remove entity Diff0 Health
data remove entity Diff0 nope
data get entity Nobody
data get entity @e[type=minecraft:pig]
execute if data entity Diff0 Health
execute if data entity Diff0 nope
! kill @e[type=!minecraft:player]
! summon minecraft:pig 5 101 5 {NoAI:1b,Silent:1b}
data get entity @e[type=minecraft:pig,limit=1] Health
data get entity @e[type=minecraft:pig,limit=1] NoAI
data merge entity @e[type=minecraft:pig,limit=1] {Health:5f}
data merge entity @e[type=minecraft:pig,limit=1] {Health:5f}
data get entity @e[type=minecraft:pig,limit=1] Health
data modify entity @e[type=minecraft:pig,limit=1] Health set value 7f
data get entity @e[type=minecraft:pig,limit=1] Health
execute store result entity @e[type=minecraft:pig,limit=1] Health float 0.5 run data get storage kiln:t a
data get entity @e[type=minecraft:pig,limit=1] Health
execute if data entity @e[type=minecraft:pig,limit=1] {NoAI:1b}

# tags
tag Diff0 list
tag @a list
tag Diff0 add a
tag Diff0 add a
tag Diff0 add b
tag Diff0 list
tag @a add a
tag @a add c
tag @a list
tag Other0 list
execute if entity @a[tag=a]
execute if entity @a[tag=b]
execute if entity @a[tag=!b]
execute if entity @a[tag=]
tag Diff0 remove a
tag Diff0 remove a
tag @a remove zz
tag @a remove c
tag @a list
tag @e[type=minecraft:pig] add pigtag
tag @e[type=minecraft:pig] list
data get entity @e[type=minecraft:pig,limit=1] Tags
execute if entity @e[tag=pigtag]
tag @e list
tag Nobody add x
tag Diff0 add "quoted"
tag Diff0 add x y
data get entity Diff0 Tags
! tag @a remove a
! tag @a remove b
! kill @e[type=!minecraft:player]
"""


def english_lang(dest: Path) -> Path:
    """Extracts en_us.json from the vanilla server jar (bundled or unpacked)."""
    out = dest / "en_us.json"
    if out.exists():
        return out
    name = "assets/minecraft/lang/en_us.json"
    inner = WORK / "versions" / VERSION / f"server-{VERSION}.jar"
    if inner.exists():
        data = zipfile.ZipFile(inner).read(name)
    else:
        with zipfile.ZipFile(WORK / "server.jar") as bundle:
            nested = bundle.open(f"META-INF/versions/{VERSION}/server-{VERSION}.jar")
            data = zipfile.ZipFile(nested).read(name)
    out.write_bytes(data)
    return out


def port_free(port: int) -> bool:
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", port)) != 0


ANSI = re.compile(r"\x1b\[[0-9;]*m")
# 26.x logs system messages to the console as "System chat: <text>" and unsigned chat (from
# /say and friends) as "[Not Secure] <text>".
VANILLA_LINE = re.compile(
    r"^\[\d\d:\d\d:\d\d\] \[Server thread/(?:INFO|WARN)\]: (?:System chat: |\[Not Secure\] )(.*)$"
)
KILN_LINE = re.compile(r"^\S+\s+(?:INFO|WARN)\s+kiln_sim::commands: (.*)$")
NOISE = re.compile(r"^Can't keep up!")


class Server:
    """A server process whose console lines are filtered to command feedback."""

    def __init__(self, name, argv, cwd, env, pattern, log):
        self.name = name
        self.pattern = pattern
        self.queue = queue.Queue()
        self.all = []
        self.log = open(log, "w", encoding="utf-8")
        self.p = subprocess.Popen(
            argv, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", bufsize=1,
        )
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.p.stdout:
            line = ANSI.sub("", raw.rstrip("\r\n"))
            self.log.write(line + "\n")
            self.log.flush()
            self.all.append(line)
            m = self.pattern.match(line)
            if m and not NOISE.match(m.group(1)):
                self.queue.put(m.group(1))

    def wait_for(self, text, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if any(text in l for l in self.all):
                return True
            if self.p.poll() is not None:
                return False
            time.sleep(0.2)
        return False

    def send(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def run(self, command, marker, timeout=20.0):
        """Runs `command`, then an unknown command `marker`; returns the lines in between."""
        self.send(command)
        self.send(marker)
        out = []
        end = time.time() + timeout
        while True:
            try:
                line = self.queue.get(timeout=max(0.01, end - time.time()))
            except queue.Empty:
                return out + [f"<timeout waiting for {self.name}>"]
            if line == f"{marker}<--[HERE]":
                if out and out[-1] == UNKNOWN:
                    out.pop()
                return out
            out.append(line)

    def stop(self):
        if self.p.poll() is None:
            try:
                self.send("stop")
                self.p.wait(timeout=60)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()
        self.log.close()


def start_vanilla(port: int) -> Server:
    base = SCRATCH / "vanilla"
    base.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(base / "world", ignore_errors=True)
    # The test functions: a world pack, found and enabled when the world is created.
    shutil.copytree(DATAPACK, base / "world" / "datapacks" / DATAPACK.name)
    (base / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    props = [
        f"server-port={port}",
        "online-mode=false",
        "white-list=false",
        "enforce-secure-profile=false",
        "level-type=minecraft\\:flat",
        "level-seed=1",
        "generate-structures=false",
        "difficulty=peaceful",
        "spawn-protection=0",
        "allow-flight=true",
        "view-distance=4",
        "simulation-distance=4",
        "max-tick-time=-1",
        "sync-chunk-writes=false",
        "enable-rcon=false",
        "enable-query=false",
    ]
    (base / "server.properties").write_text("\n".join(props) + "\n", encoding="utf-8")
    argv = ["java", "-Xmx2G", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8", "-Dfile.encoding=UTF-8",
            "-jar", str(WORK / "server.jar"), "--nogui"]
    return Server("vanilla", argv, base, os.environ.copy(), VANILLA_LINE, SCRATCH / "vanilla.log")


def start_kiln(port: int, exe: Path, lang: Path) -> Server:
    base = SCRATCH / "kiln"
    base.mkdir(parents=True, exist_ok=True)
    copy = SCRATCH / "kiln-diff.exe"
    shutil.copy2(exe, copy)
    env = os.environ.copy()
    env.update({"KILN_PORT": str(port), "KILN_LANG": str(lang), "RUST_LOG": "info", "NO_COLOR": "1"})
    env.pop("KILN_WORLD", None)
    packs = SCRATCH / "kiln-datapacks"
    shutil.rmtree(packs, ignore_errors=True)
    shutil.copytree(DATAPACK, packs / DATAPACK.name)
    env["KILN_DATAPACKS"] = str(packs)
    env.pop("KILN_OPS", None)
    return Server("kiln", [str(copy)], base, env, KILN_LINE, SCRATCH / "kiln.log")


def start_bot(port: int, exe: Path, prefix: str, log: Path):
    argv = [str(exe), "--addr", f"127.0.0.1:{port}", "--count", "1", "--behavior", "idle",
            "--duration", "3600", "--name-prefix", prefix, "--report-interval", "3600"]
    return subprocess.Popen(argv, stdout=open(log, "w", encoding="utf-8"), stderr=subprocess.STDOUT)


def parse_cases(text):
    cases, section = [], ""
    for line in text.strip().splitlines():
        line = line.strip()
        if not line:
            continue
        if line.startswith("# "):
            section = line[2:]
        elif line.startswith("!v "):
            cases.append(("vanilla", section, line[3:]))
        elif line.startswith("! "):
            cases.append(("setup", section, line[2:]))
        else:
            cases.append(("compare", section, line))
    return cases


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kiln-port", type=int, default=25587)
    ap.add_argument("--vanilla-port", type=int, default=25591)
    ap.add_argument("--kiln-exe", default=str(ROOT / "target" / "debug" / "kiln.exe"))
    ap.add_argument("--bot-exe", default=str(ROOT / "target" / "debug" / "kiln-bot.exe"))
    ap.add_argument("-k", dest="filter", help="only compare cases containing this text")
    ap.add_argument("-v", "--verbose", action="store_true", help="print every case")
    a = ap.parse_args()

    for port in (a.kiln_port, a.vanilla_port):
        if not port_free(port):
            sys.exit(f"port {port} is in use")
    SCRATCH.mkdir(parents=True, exist_ok=True)
    lang = english_lang(SCRATCH)
    bot_exe = SCRATCH / "kiln-bot-diff.exe"
    shutil.copy2(a.bot_exe, bot_exe)

    servers = [start_vanilla(a.vanilla_port), start_kiln(a.kiln_port, Path(a.kiln_exe), lang)]
    vanilla, kiln = servers
    bots = []
    try:
        if not vanilla.wait_for("Done (", 300):
            sys.exit("vanilla did not start")
        if not kiln.wait_for("listening on", 60):
            sys.exit("kiln did not start")
        seq = 0

        def run(server, command):
            nonlocal seq
            seq += 1
            return server.run(command, f"zqsync{seq}")

        # One bot at a time, so both servers list the players in the same join order.
        for prefix in ("Diff", "Other"):
            for server, port in ((vanilla, a.vanilla_port), (kiln, a.kiln_port)):
                bots.append(start_bot(port, bot_exe, prefix, SCRATCH / f"bot-{prefix}-{server.name}.log"))
                end = time.time() + 60
                while not (run(server, f"execute if entity {prefix}0") or [""])[0].startswith("Test passed"):
                    if time.time() > end:
                        sys.exit(f"bot {prefix}0 did not join {server.name}")
                    time.sleep(0.5)

        passed, failed = 0, []
        for kind, section, command in parse_cases(CASES):
            if kind == "vanilla":
                run(vanilla, command)
                time.sleep(2)  # let forced chunks load and generate
                continue
            if kind == "setup":
                for server in servers:
                    run(server, command)
                continue
            if a.filter and a.filter not in command:
                # Still run it so later cases see the same world.
                for server in servers:
                    run(server, command)
                continue
            want, got = run(vanilla, command), run(kiln, command)
            ok = want == got
            if ok:
                passed += 1
            else:
                failed.append((section, command, want, got))
            if a.verbose or not ok:
                print(f"{'PASS' if ok else 'FAIL'} [{section}] {command}")
                if not ok or a.verbose:
                    for line in want:
                        print(f"    vanilla: {line}")
                    for line in got:
                        print(f"    kiln:    {line}")
        total = passed + len(failed)
        print(f"\n{passed}/{total} command lines match vanilla {VERSION}")
        by_section = {}
        for section, *_ in failed:
            by_section[section] = by_section.get(section, 0) + 1
        for section, n in by_section.items():
            print(f"  {n} mismatches in {section}")
        return 0 if not failed else 1
    finally:
        for b in bots:
            b.kill()
        for server in servers:
            server.stop()


if __name__ == "__main__":
    sys.exit(main())
