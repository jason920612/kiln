"""Differential command test: run the same console command lines on the vanilla 26.3 server
and on Kiln, and compare the console feedback line by line.

usage: python tools/command_diff.py [--kiln-port 25587] [--vanilla-port 25591] [-k TEXT] [-v]

Needs KILN_WORK (default <repo>/work) with the vanilla server.jar and versions/26.3/ (scratch
files go to KILN_DIFF_SCRATCH, default $KILN_WORK/wp2-commands/diff), and
built executables (cargo build -p kiln-server -p kiln-bot). Both servers get a flat world and
two idle kiln-bots ("Diff0", then "Other0") for selectors. Kiln prints English on its
console through KILN_LANG (en_us.json extracted from the vanilla jar into the scratch dir).

Case syntax (CASES below): one command per line; "# ..." starts a section; "! cmd" runs on
both servers without comparing; "!v cmd" runs on vanilla only (setup Kiln lacks), "~ N" waits N seconds and then compares the
console lines that arrived meanwhile (results that come over ticks or from other threads).
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
# A zip pack (vanilla reads `.zip` packs in the world's datapacks directory in place).
ZIP_PACK = {
    "pack.mcmeta": '{"pack":{"description":"zipped","min_format":121,"max_format":121}}',
    "data/kilnzip/function/hi.mcfunction": "say hello from a zip\n",
}
# Both worlds are created with a feature pack enabled (`initial-enabled-packs`).
INITIAL_PACKS = "vanilla,minecart_improvements"
UNKNOWN = "Unknown or incomplete command. See below for error"

# Test area: chunks -1..1 around 0,0, y 100..170, cleared to air first. Vanilla's flat world
# spawns animals as chunks generate and drops items for `destroy`; they are killed before the
# sections that select entities (Kiln has neither). Periodic animal spawns are turned off.
CASES = r"""
! gamerule spawn_mobs false
! forceload add -32 -32 47 47
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
datapack enable trade_rebalance
datapack enable redstone_experiments
datapack list available
function kilnzip:hi
datapack disable "file/kilnzip.zip"
function kilnzip:hi
datapack enable "file/kilnzip.zip"
function kilnzip:hi
function kilndiff:ent with entity Diff0
function kilndiff:abil with entity Diff0 abilities
function kilndiff:ent with entity Diff0 abilities
function kilndiff:macro with entity Diff0
function kilndiff:ent with entity Diff0 Air
function kilndiff:ent with entity Diff0 nope
function kilndiff:ent with entity @a
function kilndiff:ent with entity Nobody
execute as Other0 run function kilndiff:ent with entity @s
! summon minecraft:pig 8 101 8 {NoAI:1b,Invulnerable:1b}
function kilndiff:ent with entity @e[type=minecraft:pig,limit=1]
! kill @e[type=minecraft:pig]
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

# whitelist
whitelist list
whitelist add Diff0
whitelist add Diff0
whitelist add @a
whitelist list
whitelist on
whitelist on
whitelist reload
whitelist remove Other0
whitelist remove Other0
whitelist add Zqx9NoAcct1
whitelist remove Zqx9NoAcct1
whitelist add @e[type=minecraft:pig]
whitelist list
whitelist off
whitelist off
whitelist remove @a
whitelist list
whitelist bogus

# bans
banlist
banlist players
banlist ips
ban Zqx9NoAcct1
ban Zqx9NoAcct2 griefing a lot
pardon Zqx9NoAcct1
ban @e[type=minecraft:pig]
banlist players
ban-ip 10.0.0.1
ban-ip 10.0.0.1
ban-ip 10.0.0.2 spam
ban-ip 192.168.1.20 a b  c
ban-ip 172.16.0.1
ban-ip notanip
ban-ip 10.0.0.300
ban-ip ::1
ban-ip Zqx9NoAcct1
banlist ips
banlist
banlist players
pardon-ip 10.0.0.1
pardon-ip 10.0.0.1
pardon-ip notanip
pardon-ip 10.0.0.2
pardon-ip ::1
banlist
pardon-ip 192.168.1.20
pardon-ip 172.16.0.1
banlist
pardon @a
banlist bogus

# server settings
save-off
save-off
save-on
save-on
save-all
save-all flush
defaultgamemode survival
defaultgamemode creative
defaultgamemode adventure
defaultgamemode survival
defaultgamemode bogus
gamemode creatve
gamemode creatve Diff0
setidletimeout 0
setidletimeout 10
setidletimeout 0
setidletimeout -1
! version
publish
publish true
publish false 25599
unpublish
jfr stop
perf stop
debug stop

# particles and sounds
particle minecraft:flame
particle minecraft:flame 8 160 8
particle flame 0 100 0
particle flame 0 100 0 1 1 1 0.5 10
particle flame 0 100 0 1 1 1 0.5 10 force
particle flame 8 160 8 1 1 1 0.5 10 normal
particle flame 8 160 8 1 1 1 0.5 10 normal Diff0
particle flame 8 160 8 1 1 1 0.5 10 force @a
particle minecraft:dust{color:[1.0,0.0,0.0],scale:1.0} 8 160 8
particle minecraft:dust 8 160 8
particle minecraft:block{block_state:"minecraft:stone"} 8 160 8
particle minecraft:nope 8 160 8
particle flame 8 160 8 1 1 1 0.5 -1
playsound minecraft:entity.pig.ambient master Diff0
playsound minecraft:entity.pig.ambient master @a
playsound minecraft:entity.pig.ambient master @a 8 160 8
playsound minecraft:entity.pig.ambient master @a 0 100 0
playsound minecraft:entity.pig.ambient master @a 0 100 0 1 1 0.5
playsound minecraft:entity.pig.ambient master @a 0 100 0 1 1 1
playsound minecraft:entity.pig.ambient music Diff0 8 160 8 2 0.5
playsound minecraft:entity.pig.ambient bogus Diff0
playsound minecraft:not.a.sound master Diff0
playsound minecraft:entity.pig.ambient master Nobody
playsound minecraft:entity.pig.ambient master @a ~ ~ ~ 1 3
playsound minecraft:entity.pig.ambient master @a ~ ~ ~ 1 1 2
stopsound Diff0
stopsound @a
stopsound Diff0 master
stopsound Diff0 * minecraft:entity.pig.ambient
stopsound Diff0 music minecraft:entity.pig.ambient
stopsound Nobody
stopsound Diff0 bogus

# stopwatch
stopwatch create kiln:sw
stopwatch create kiln:sw
stopwatch restart kiln:sw
stopwatch restart kiln:nope
stopwatch query kiln:nope
stopwatch remove kiln:sw
stopwatch remove kiln:sw
stopwatch create sw2
stopwatch remove sw2

# post effects and waypoints
posteffect list Diff0
posteffect add Diff0 minecraft:creeper
posteffect add Diff0 minecraft:creeper
posteffect add @a minecraft:spider
posteffect list Diff0
posteffect list Other0
posteffect remove Diff0 minecraft:creeper
posteffect remove Diff0 minecraft:creeper
posteffect remove @a minecraft:spider
posteffect clear @a
posteffect clear Diff0
posteffect list Diff0
waypoint list
waypoint modify Diff0 color red
waypoint modify Diff0 color hex FF00AA
waypoint modify Diff0 color reset
waypoint modify Diff0 style set minecraft:bowtie
waypoint modify Diff0 style reset
waypoint modify Diff0 color hex F0A
waypoint modify Diff0 color hex GGGGGG
waypoint modify Diff0 color hex 12345
waypoint modify Diff0 color blurple
waypoint modify Other0 color reset
waypoint modify Nobody color reset
waypoint modify @a color reset
waypoint modify Diff0 color hex 1G2345
waypoint modify Diff0 color hex +F0000
waypoint list
execute in minecraft:the_nether run waypoint list
gamerule locator_bar false
waypoint list
gamerule locator_bar true
! summon minecraft:pig 8 160 8
waypoint modify @e[type=minecraft:pig,limit=1] color red
! kill @e[type=minecraft:pig]
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

# rotate, swing, spectate, ride
! gamemode survival @a
! summon minecraft:pig 5 101 5 {NoAI:1b,Silent:1b,Tags:["p1"]}
! summon minecraft:pig 7 101 5 {NoAI:1b,Silent:1b,Tags:["p2"]}
rotate Diff0 90 0
rotate Diff0 ~10 ~
rotate Diff0 facing 0 100 0
rotate Diff0 facing entity Other0
rotate Diff0 facing entity Other0 eyes
rotate Nobody 0 0
rotate @a 0 0
rotate @e[tag=p1,limit=1] 45 10
data get entity @e[tag=p1,limit=1] Rotation
rotate @e[tag=p1,limit=1] facing 5 101 10
data get entity @e[tag=p1,limit=1] Rotation[0]
swing Diff0
swing @a mainhand
swing @a offhand stab
swing @a offhand whack 10
swing @e[type=minecraft:pig]
swing @e[type=minecraft:pig,limit=1] mainhand none
swing Diff0 mainhand bogus
swing Diff0 mainhand whack 0
swing
spectate
spectate Diff0
spectate Diff0 Other0
spectate Other0 Other0
! gamemode spectator Other0
spectate Diff0 Other0
spectate Other0 Other0
spectate @e[tag=p1,limit=1] Other0
! gamemode survival Other0
ride Diff0 dismount
ride Diff0 mount @e[tag=p1,limit=1]
ride Diff0 mount @e[tag=p1,limit=1]
ride Diff0 mount @e[tag=p2,limit=1]
ride Diff0 dismount
ride Diff0 dismount
ride @e[tag=p1,limit=1] mount Diff0
ride @e[tag=p1,limit=1] mount @e[tag=p1,limit=1]
ride @e[tag=p2,limit=1] mount @e[tag=p1,limit=1]
ride @e[tag=p1,limit=1] mount @e[tag=p2,limit=1]
ride @e[tag=p2,limit=1] dismount
ride Nobody dismount
ride @e[type=minecraft:pig] dismount

# clear and enchant
! clear @a
clear Diff0
clear @a
clear Diff0 minecraft:stone
! give Diff0 minecraft:stone 10
! give Diff0 minecraft:dirt 5
! give Other0 minecraft:stone 2
clear Diff0 minecraft:stone 0
clear @a minecraft:stone 0
clear Diff0 minecraft:stone 3
clear Diff0 minecraft:stone 0
clear @a minecraft:stone 1
clear @a minecraft:stone
clear @a minecraft:stone
! give Diff0 minecraft:oak_log 2
clear Diff0 #minecraft:logs 0
clear Diff0 #minecraft:logs
clear Diff0 * 0
clear @a * 0
clear Diff0
clear Diff0 minecraft:nonexistent
clear Diff0 #minecraft:nonexistent
clear @e[type=minecraft:pig]
! give Diff0 minecraft:diamond_sword
enchant Diff0 minecraft:sharpness
enchant Diff0 minecraft:sharpness 3
enchant Diff0 minecraft:smite
enchant Diff0 minecraft:unbreaking 10
enchant Diff0 minecraft:unbreaking 2
enchant @a minecraft:mending
enchant Other0 minecraft:mending
enchant @e[tag=p1,limit=1] minecraft:mending
enchant @e[type=minecraft:pig] minecraft:mending
enchant Diff0 minecraft:nonexistent
enchant Diff0 minecraft:binding_curse
enchant Diff0 minecraft:fire_aspect 0
data get entity Diff0 SelectedItem.components."minecraft:enchantments"."minecraft:sharpness"
data get entity Diff0 SelectedItem.components."minecraft:enchantments"."minecraft:unbreaking"
! clear @a
! give Diff0 minecraft:stick
enchant Diff0 minecraft:sharpness
! give Diff0 minecraft:book
! clear Diff0 minecraft:stick
enchant Diff0 minecraft:efficiency
! clear @a
! kill @e[type=!minecraft:player]

# attributes and damage
! summon minecraft:pig 5 101 5 {NoAI:1b,Silent:1b,Tags:["p1"]}
attribute Diff0 minecraft:max_health get
attribute Diff0 minecraft:max_health get 2.5
attribute Diff0 minecraft:movement_speed get
attribute Diff0 minecraft:movement_speed get 100
attribute Diff0 minecraft:attack_speed base get
attribute Diff0 minecraft:block_interaction_range base get
attribute Diff0 minecraft:luck get
attribute Diff0 minecraft:scale get
attribute Diff0 minecraft:gravity base get 1000
attribute Diff0 minecraft:follow_range get
attribute Diff0 minecraft:nonexistent get
attribute Diff0 minecraft:max_health base set 30
attribute Diff0 minecraft:max_health get
attribute Diff0 minecraft:max_health base reset
attribute Diff0 minecraft:max_health get
attribute Diff0 minecraft:attack_damage modifier add kiln:boost 2 add_value
attribute Diff0 minecraft:attack_damage modifier add kiln:boost 2 add_value
attribute Diff0 minecraft:attack_damage get
attribute Diff0 minecraft:attack_damage modifier add kiln:mul 0.5 add_multiplied_base
attribute Diff0 minecraft:attack_damage modifier add kiln:tot 1 add_multiplied_total
attribute Diff0 minecraft:attack_damage get
attribute Diff0 minecraft:attack_damage modifier value get kiln:boost
attribute Diff0 minecraft:attack_damage modifier value get kiln:mul 10
attribute Diff0 minecraft:attack_damage modifier value get kiln:nope
attribute Diff0 minecraft:attack_damage modifier remove kiln:boost
attribute Diff0 minecraft:attack_damage modifier remove kiln:boost
attribute Diff0 minecraft:attack_damage modifier remove kiln:mul
attribute Diff0 minecraft:attack_damage modifier remove kiln:tot
attribute Diff0 minecraft:attack_damage get
attribute @e[tag=p1,limit=1] minecraft:max_health get
attribute @e[tag=p1,limit=1] minecraft:max_health base get
attribute @e[tag=p1,limit=1] minecraft:movement_speed get
attribute @e[tag=p1,limit=1] minecraft:movement_speed base set 0.5
attribute @e[tag=p1,limit=1] minecraft:movement_speed get
attribute @e[tag=p1,limit=1] minecraft:movement_speed base reset
attribute @e[tag=p1,limit=1] minecraft:max_health modifier add kiln:hp 4 add_value
attribute @e[tag=p1,limit=1] minecraft:max_health get
attribute @e[tag=p1,limit=1] minecraft:max_health modifier remove kiln:hp
attribute @e[tag=p1,limit=1] minecraft:attack_damage get
attribute @a minecraft:max_health get
attribute Nobody minecraft:max_health get
! gamerule natural_health_regeneration false
damage Diff0 2
damage Diff0 1 minecraft:nonexistent
damage Nobody 1
data get entity Diff0 Health
! gamerule natural_health_regeneration true
# damage on other entities (wp34): any damage type, at a position, by and from entities
! summon cow 3 100 3 {Tags:["dmg1"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
! summon cow 5 100 3 {Tags:["dmg2"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
! summon cow 7 100 3 {Tags:["dmg3"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
! summon cow 9 100 3 {Tags:["dmg4"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b,Invulnerable:1b}
! summon cow 11 100 3 {Tags:["dmg5"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
! summon cow 13 100 3 {Tags:["dmg6"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
! summon zombie 15 100 3 {Tags:["dmg7"],NoAI:1b,NoGravity:1b,Health:20f,PersistenceRequired:1b}
! summon pig 17 100 3 {Tags:["dmg8"],NoAI:1b,NoGravity:1b,Health:10f,PersistenceRequired:1b}
damage @e[tag=dmg1,limit=1] 3
data get entity @e[tag=dmg1,limit=1] Health
damage @e[tag=dmg2,limit=1] 4 minecraft:fall by Diff0
data get entity @e[tag=dmg2,limit=1] Health
damage @e[tag=dmg3,limit=1] 1.5 minecraft:magic at 7 100 3
data get entity @e[tag=dmg3,limit=1] Health
damage @e[tag=dmg4,limit=1] 2 minecraft:generic
data get entity @e[tag=dmg4,limit=1] Health
damage @e[tag=dmg4,limit=1] 2 minecraft:out_of_world
damage @e[tag=dmg5,limit=1] 2.5 minecraft:spear by Other0 from Diff0
data get entity @e[tag=dmg5,limit=1] Health
damage @e[tag=dmg6,limit=1] 0 minecraft:generic
damage @e[tag=dmg6,limit=1] 1 minecraft:lava
data get entity @e[tag=dmg6,limit=1] Health
damage @e[tag=dmg7,limit=1] 3 minecraft:cactus
data get entity @e[tag=dmg7,limit=1] Health
damage @e[tag=dmg8,limit=1] 100 minecraft:generic_kill
damage @e[tag=dmg8,limit=1] 1
damage @e[tag=dmg8] 1 minecraft:generic by Diff0
! kill @e[type=!minecraft:player]
! kill @e[type=!minecraft:player]
# worldborder
worldborder get
worldborder set 100
worldborder set 100
worldborder get
worldborder set 0.5
worldborder set 60000000
worldborder set 59999969
worldborder add 20
worldborder add -10
worldborder get
worldborder set 200 20
! worldborder set 100
worldborder add 10 5s
! worldborder set 100
worldborder set 50 1d
! worldborder set 100
worldborder add -1000
worldborder center 10 20
worldborder center 10 20
worldborder center 10.5 20.25
worldborder center 30000000 0
worldborder center 0 0
worldborder damage amount 0.5
worldborder damage amount 0.5
worldborder damage amount -1
worldborder damage amount 0.2
worldborder damage buffer 2.125
worldborder damage buffer 2.125
worldborder damage buffer 5
worldborder warning distance 10
worldborder warning distance 10
worldborder warning distance -1
worldborder warning distance 5
worldborder warning time 20s
worldborder warning time 15
worldborder warning time 15
worldborder warning time 300
worldborder
worldborder set
execute store result score #b fn run worldborder get
worldborder set 59999968
worldborder get

# tick
tick rate 20
tick rate 40
tick rate 0.5
tick rate 10001
tick rate 20
tick step
tick step stop
tick sprint stop
tick freeze
tick step
tick step 100
tick step stop
tick step stop
tick step 0
tick unfreeze
tick bogus

# forceload
forceload query
forceload query 0 0
forceload query 100 100
forceload add 100 100
forceload add 100 100
forceload query 100 100
forceload add 96 96 130 130
forceload query
forceload remove 100 100
forceload remove 100 100
forceload add 0 0 1000 1000
forceload add 30000000 0
forceload add -30000001 0
forceload remove 96 96 130 130
execute in minecraft:the_nether run forceload query
execute in minecraft:the_nether run forceload add 0 0
execute in minecraft:the_nether run forceload query
execute in minecraft:the_nether run forceload remove all
forceload query

# random
random value 1..1
random value ..5
random value 5
! random reset *
random reset *
random reset kiln:a 42 false true
random value 1..1000 kiln:a
random value 1..1000 kiln:a
random roll 1..6 kiln:a
random reset kiln:b 7 false false
random value 1..100 kiln:b
random reset kiln:b 7 false false
random value 1..100 kiln:b
random reset * 3 false true
random value 1..100 kiln:c
random value -50..50 kiln:c
random reset *
random value 1..2147483647 kiln:d
random reset kiln:bad:id
random bogus

# locate
execute positioned 0 -60 0 run locate biome minecraft:plains
execute positioned 0 -60 0 run locate biome #minecraft:is_overworld
locate biome minecraft:desert
locate biome #minecraft:is_nether
locate biome minecraft:nonexistent
locate structure minecraft:nonexistent
locate structure #minecraft:nonexistent
locate poi minecraft:librarian
! setblock 3 110 3 lectern
execute positioned 0 100 0 run locate poi minecraft:librarian
execute positioned 0 100 0 run locate poi #minecraft:acquirable_job_site
locate poi minecraft:nonexistent
! setblock 3 110 3 air
execute positioned 100 100 100 run locate biome minecraft:plains
execute in minecraft:the_nether run locate biome minecraft:plains

# fillbiome
fillbiome 0 100 0 15 110 15 minecraft:desert
fillbiome 0 100 0 15 110 15 minecraft:desert
execute if biome 4 104 4 minecraft:desert
execute if biome 4 96 4 minecraft:desert
fillbiome 0 100 0 15 110 15 minecraft:plains replace minecraft:desert
fillbiome 0 100 0 15 110 15 minecraft:plains replace #minecraft:is_ocean
fillbiome 2 101 2 3 101 3 minecraft:badlands
fillbiome 0 100 0 15 110 15 minecraft:plains
fillbiome 0 0 0 1000 10 1000 minecraft:desert
fillbiome 100000 100 0 100001 100 0 minecraft:desert
fillbiome 0 100 0 1 100 1 minecraft:nonexistent
fillbiome 0 400 0 1 400 1 minecraft:desert

# place
place feature minecraft:nonexistent
place template minecraft:nonexistent 0 120 0
place template minecraft:igloo/top 0 120 0 bogus
place jigsaw minecraft:nonexistent minecraft:x 1
place jigsaw minecraft:village/plains/town_centers minecraft:x 21
place structure minecraft:nonexistent
place template minecraft:igloo/top 100000 120 0

# spreadplayers
spreadplayers 0 0 1 10 false Diff0
spreadplayers 0 0 1 10 true Diff0
spreadplayers 0 0 1 0.5 false Diff0
spreadplayers 0 0 -1 10 false Diff0
spreadplayers 0 0 1 10 under -100 false Diff0
spreadplayers 0 0 1 10 false @e[type=minecraft:pig]
spreadplayers 0 0 1 10 false
! tp Diff0 8 160 8 0 0

# loot
! clear @a
! setblock 6 100 6 stone
! setblock 7 100 6 chest
! setblock 8 100 6 air
loot give Diff0 mine 6 100 6
loot give Diff0 mine 6 100 6 minecraft:diamond_pickaxe[minecraft:enchantments={"minecraft:silk_touch":1}]
loot give @a mine 6 100 6
loot give Diff0 mine 8 100 6
loot give Diff0 mine 6 100 6 mainhand
execute as Diff0 run loot give Diff0 mine 6 100 6 mainhand
loot give Diff0 loot minecraft:blocks/stone
loot give Diff0 loot minecraft:blocks/dirt
loot give Diff0 loot minecraft:nonexistent
loot give Nobody loot minecraft:blocks/stone
loot insert 7 100 6 loot minecraft:blocks/stone
loot insert 7 100 6 loot minecraft:blocks/dirt
loot insert 7 100 6 mine 6 100 6
data get block 7 100 6 Items
loot insert 6 100 6 loot minecraft:blocks/stone
loot replace block 7 100 6 container.3 loot minecraft:blocks/dirt
loot replace block 7 100 6 container.30 loot minecraft:blocks/dirt
loot replace block 7 100 6 container.4 2 loot minecraft:blocks/dirt
loot replace block 7 100 6 container.5 0 loot minecraft:blocks/dirt
data get block 7 100 6 Items
loot replace entity Diff0 hotbar.8 loot minecraft:blocks/cobblestone
loot replace entity @a armor.head loot minecraft:blocks/carved_pumpkin
data get entity Diff0 Inventory[{Slot:8b}].id
loot spawn 5 101 5 loot minecraft:blocks/stone
loot spawn 5 101 5 mine 6 100 6
loot give Diff0 kill Other0
loot give Diff0 kill @e[type=minecraft:item,limit=1]
loot give Diff0 loot {pools:[{rolls:1,entries:[{type:"minecraft:item",name:"minecraft:apple"}]}]}
loot give Diff0 fish minecraft:blocks/stone 6 100 6
! clear @a
! kill @e[type=!minecraft:player]

# item
! gamerule show_advancement_messages false
! setblock 6 100 6 stone
! setblock 7 100 6 air
! setblock 7 100 6 chest
! setblock 8 100 6 furnace
item replace block 7 100 6 container.0 with minecraft:diamond 5
item replace block 7 100 6 container.1 with minecraft:stone 100
item replace block 7 100 6 container.1 with minecraft:stone 64
item replace block 7 100 6 container.1 with minecraft:air
item replace block 7 100 6 container.30 with minecraft:dirt
item replace block 7 100 6 container.99 with minecraft:dirt
item replace block 6 100 6 container.0 with minecraft:dirt
item replace block 9 100 6 container.0 with minecraft:dirt
item replace block 7 100 6 container.* with minecraft:apple
item replace block 7 100 6 container.* with minecraft:apple 2
item fill block 7 100 6 container.* with minecraft:cobblestone 3
item override block 7 100 6 container.* with minecraft:dirt
item override block 7 100 6 container.3 with minecraft:air
data get block 7 100 6 Items
item replace block 8 100 6 container.1 with minecraft:coal 5
item replace block 8 100 6 container.3 with minecraft:coal 5
item replace block 8 100 6 container.* with minecraft:coal 5
data get block 8 100 6 Items
item replace block 7 100 6 foo.bar with minecraft:dirt
item replace block 7 100 6 kilndiff:nothing with minecraft:dirt
item replace block 7 100 6 kilndiff:first_two with minecraft:emerald 2
item fill block 7 100 6 kilndiff:first_two with minecraft:emerald 2
item override block 7 100 6 kilndiff:both with minecraft:gold_ingot
item replace block 7 100 6 {type:"minecraft:slot_range",slots:"container.7"} with minecraft:iron_ingot
data get block 7 100 6 Items
item modify block 7 100 6 container.0 kilndiff:three
item modify block 7 100 6 container.* kilndiff:three
item modify block 7 100 6 container.* kilndiff:nothing
item modify block 6 100 6 container.0 kilndiff:three
item modify block 7 100 6 container.0 {type:"minecraft:set_count",count:7}
data get block 7 100 6 Items
item replace block 7 100 6 container.20 with minecraft:air
item modify block 7 100 6 container.20 kilndiff:three
item replace entity Diff0 hotbar.0 with minecraft:golden_apple 2
item replace entity Diff0 hotbar.1 with minecraft:stone 65
item replace entity Diff0 hotbar.* with minecraft:stick
item replace entity Diff0 hotbar.* with minecraft:stick 3
item fill entity Diff0 hotbar.* with minecraft:arrow
item override entity Diff0 hotbar.* with minecraft:bow
item override entity Diff0 hotbar.4 with minecraft:air
data get entity Diff0 Inventory
item replace entity @a hotbar.1 from entity Diff0 hotbar.0
item replace entity Other0 hotbar.* from entity Diff0 hotbar.*
item replace entity Diff0 hotbar.* from block 7 100 6 container.*
item replace entity Diff0 hotbar.2 from block 7 100 6 container.0 kilndiff:three
item replace entity Diff0 armor.head with minecraft:iron_helmet
item replace entity Diff0 armor.* with minecraft:diamond_chestplate
item replace entity Diff0 weapon.offhand with minecraft:shield
item replace entity Diff0 weapon.mainhand with minecraft:stone
item replace entity Diff0 inventory.5 with minecraft:cake
item replace entity Diff0 enderchest.3 with minecraft:cake
item replace entity Diff0 horse.3 with minecraft:cake
item replace entity Diff0 contents with minecraft:cake
item replace entity Nobody hotbar.0 with minecraft:cake
item replace entity @e[type=minecraft:pig] hotbar.0 with minecraft:cake
item replace block 7 100 6 container.0 from entity Diff0 hotbar.0
item replace block 7 100 6 container.0 from entity Diff0 horse.3
item replace block 7 100 6 container.0 from entity Diff0 hotbar.* kilndiff:three
item replace block 7 100 6 container.* from entity Diff0 armor.*
item replace block 7 100 6 container.0 from block 8 100 6 container.0
item replace block 7 100 6 container.0 from block 6 100 6 container.0
item replace block 7 100 6 container.0 from block 8 100 6 container.20
item replace block 7 100 6 container.0 from entity Nobody hotbar.0
item replace entity Diff0 hotbar.0 from entity Diff0 hotbar.0 kilndiff:nothing
item modify entity Diff0 hotbar.0 kilndiff:three
item modify entity @a hotbar.* kilndiff:three
item modify entity Diff0 hotbar.8 kilndiff:three
item modify entity Diff0 armor.* kilndiff:name
item modify entity Nobody hotbar.0 kilndiff:three
item modify entity Diff0 hotbar.0 kilndiff:missing
item replace entity Diff0 kilndiff:hotbar_apples with minecraft:carrot
item replace entity Diff0 kilndiff:first_two with minecraft:carrot
item modify entity Diff0 kilndiff:hotbar_apples kilndiff:three
data get entity Diff0 Inventory
item replace entity Diff0 hotbar.* with minecraft:apple 1
item replace entity Diff0 hotbar.3 with minecraft:carrot
item modify entity Diff0 kilndiff:hotbar_apples kilndiff:three
item replace entity Diff0 kilndiff:hotbar_apples with minecraft:air
data get entity Diff0 Inventory
item modify entity Diff0 hotbar.0 {type:"minecraft:bogus"}
item modify entity Diff0 hotbar.0 {}
item modify entity Diff0 hotbar.0 [1]
item modify entity Diff0 hotbar.0 "kilndiff:three"
item modify entity Diff0 hotbar.0 {type:"minecraft:set_count",count:{type:"minecraft:bogus"}}
execute if slots entity Diff0 {type:"minecraft:bogus"}
execute if slots entity Diff0 {}
execute if slots entity Diff0 {type:"minecraft:empty"}
execute if slots entity Diff0 [{type:"minecraft:empty"}]
loot give Diff0 loot {pools:[{rolls:1,entries:[{type:"minecraft:bogus"}]}]}
loot give Diff0 loot {}
item
item replace
item replace block
item replace block 7 100 6
item replace block 7 100 6 container.0
item replace block 7 100 6 container.0 with
item replace block 7 100 6 container.0 with minecraft:dirt 0
item replace block 7 100 6 container.0 with minecraft:dirt 100
item replace block 7 100 6 container.0 with minecraft:nonexistent
item bogus

# execute if items
execute if items entity Diff0 hotbar.* minecraft:apple
execute if items entity Diff0 hotbar.* minecraft:bow
execute if items entity Diff0 hotbar.0 *
execute if items entity Diff0 hotbar.8 minecraft:air
execute if items entity Diff0 hotbar.* #minecraft:logs
execute if items entity @a hotbar.* minecraft:apple
execute if items entity Nobody hotbar.0 *
execute if items entity Diff0 horse.3 *
execute if items entity Diff0 kilndiff:hotbar_apples *
execute unless items entity Diff0 hotbar.* minecraft:apple
execute unless items entity Diff0 hotbar.* minecraft:bow
execute store result score Diff0 kilndiff run execute if items entity Diff0 hotbar.* *
execute if items block 7 100 6 container.* *
execute if items block 7 100 6 container.0 minecraft:dirt
execute if items block 6 100 6 container.0 *
execute unless items block 7 100 6 container.* minecraft:emerald
execute if items block 7 100 6 kilndiff:both *
execute if items block 7 100 6 foo *

# execute if slots
execute if slots entity Diff0 hotbar.*
execute if slots entity Diff0 hotbar.3
execute if slots entity Diff0 horse.3
execute if slots entity @a hotbar.*
execute if slots entity Nobody hotbar.*
execute if slots entity Diff0 kilndiff:hotbar_apples
execute unless slots entity Diff0 kilndiff:hotbar_apples
execute if slots block 7 100 6 container.*
execute if slots block 7 100 6 kilndiff:first_two
execute if slots block 7 100 6 container.40
execute if slots block 6 100 6 container.*
execute unless slots block 7 100 6 container.99
! gamerule show_advancement_messages true
! gamerule show_advancement_messages true

# compute
compute default integer 5
compute default integer -3
compute default integer minecraft:cooking/time_coal
compute default integer minecraft:cooking/time_wool
compute default integer minecraft:compostable/always_add_one
compute default integer minecraft:nonexistent
compute default integer nonexistent
compute default float 2.5
compute default float 2.5 2
compute default float 2.5 -3
compute default float 3 1
compute default float 0.1 10
compute default float minecraft:cooking/normal_speed_multiplier
compute default float minecraft:cooking/normal_speed_multiplier 100
compute default float minecraft:cooking/nothing
compute default float {type:"minecraft:constant",value:1.5}
compute default float {type:"minecraft:uniform",min:1,max:1}
compute default integer {type:"minecraft:constant",value:7}
compute default integer {type:"minecraft:add",inputs:[1,2,3]}
compute default integer {type:"minecraft:div",left:1,right:0}
compute default integer {type:"minecraft:mul",inputs:[100000,100000]}
compute default float {type:"minecraft:div",left:1.0,right:0.0}
compute default integer {type:"minecraft:abs",input:-2147483648}
compute default integer {type:"minecraft:negate",input:-2147483648}
compute default integer {type:"minecraft:sub",left:-2147483648,right:1}
compute default integer {type:"minecraft:mod",left:1,right:0}
compute default integer {type:"minecraft:floor_div",left:1,right:0}
compute default integer {type:"minecraft:floor_div",left:-2147483648,right:-1}
compute default integer {type:"minecraft:floor_mod",left:1,right:0}
compute default integer {type:"minecraft:pow",base:0,exponent:0}
compute default integer {type:"minecraft:pow",base:2,exponent:40}
compute default integer {type:"minecraft:pow",base:2,exponent:-1}
compute default integer {type:"minecraft:avg",inputs:[]}
compute default integer {type:"minecraft:add",inputs:[2147483647,1]}
compute default integer {type:"minecraft:from_float",input:{type:"minecraft:div",left:1.0,right:0.0}}
compute default integer {type:"minecraft:from_float",input:1.0e20}
compute default float {type:"minecraft:div",left:0.0,right:0.0}
compute default float {type:"minecraft:sqrt",input:-1.0}
compute default float {type:"minecraft:div",left:-1.0,right:0.0}
compute default float {type:"minecraft:from_int",input:{type:"minecraft:pow",base:2,exponent:40}}
compute default integer {type:"minecraft:bogus"}
compute default integer {}
compute default integer "minecraft:cooking/time_coal"
compute default integer [1]
compute block 7 100 6 integer 3
compute block 7 100 6 integer minecraft:compostable/always_add_one
compute block 6 100 6 float 0.5 10
compute block 100000 100 0 integer 1
compute block 0 -100 0 integer 1
compute entity Diff0 integer 4
compute entity Diff0 float 4.5 2
compute entity Nobody integer 4
compute entity @a integer 4
compute
compute default
compute default float
compute default integer
compute bogus
! clear @a
! kill @e[type=!minecraft:player]
# test: arguments and errors
! setworldspawn 0 -60 0
test
test bogus
test clearall
test clearall 5
test clearall 100000
test clearall -3
test clearthese
test clearthat
test create
test create kilndiff:box 49
test create kilndiff:box 5 5 49
test create kilndiff:box 3 4 5
test locate kilndiff:*
test locate kilndiff:nope
test locate *
test locate
test pos
test pos x
test resetclosest
test resetthese
test resetthat
test run
test run kilndiff:nope
test run kilndiff:pass_fn -1
test runfailed
test runfailed true
test runmultiple kilndiff:nope
test runthat
test runthese
test runclosest
test stop
test verify kilndiff:nope
test verify
test run kilndiff:pass_fn abc
test run kilndiff:pass_fn 1 maybe
test run kilndiff:pass_fn 1 true -1
test run kilndiff:pass_fn 1 true 4
test run kilndiff:pass_fn 1 true 99
test run kilndiff:pass_fn 1 true 1 -1
~ 4
test run kilndiff:pass_fn 1 true 1 1 extra
test run *
~ 4
test runmultiple
test runmultiple kilndiff:pass_fn x
test runmultiple kilndiff:pass_fn 1 x
test runfailed maybe
test runfailed true x
test runfailed true 2 maybe
test locate kilndiff:pass_fn extra
test create kilndiff:box 0
test create kilndiff:box 1 1
test create kilndiff:box 1 1 0
test create kilndiff:box 1 1 1 1
test pos extra
test pos kilnx
test run kilndiff:pass_fn 1 true 3
~ 3
test run kilndiff:pass_fn 2 false 1 1
~ 4
test clearall
test run kilndiff:missing_structure
test clearall

# test: create, locate, reset and clear
test create kilndiff:pass_fn 4
test locate kilndiff:pass_fn
test locate kilndiff:*
test runclosest
~ 3
test resetclosest
test resetthese
test clearthese
test clearall
test create kilndiff:pass_fn 4 3 2
test runthese 2
~ 4
test clearall 10
test clearall

# test: runs
test run minecraft:always_pass
~ 3
test run kilndiff:pass_fn
~ 3
test run kilndiff:optional_fn
~ 3
test run kilndiff:accept
~ 3
test run kilndiff:fail
~ 3
test run kilndiff:timeout
~ 4
test run kilndiff:timeout_optional
~ 4
test run kilndiff:nostart
~ 3
test run kilndiff:rotated
~ 3
test run kilndiff:padded
~ 3
test run kilndiff:with_rules
~ 3
test run kilndiff:with_inline_env
~ 3
test run kilndiff:flaky
~ 3
test run kilndiff:slow/one
~ 3
test run kilndiff:slow/two
~ 3
test runfailed
~ 3
test run kilndiff:fail
~ 3
test runfailed
~ 3
test runfailed true
~ 3
test run kilndiff:pass_fn 3
~ 4
test run kilndiff:pass_fn 2 true
~ 4
test run kilndiff:fail 3 true
~ 4
test run kilndiff:pass_fn 1 false 1
~ 3
test run kilndiff:accept 1 false 0 1
~ 3
test runmultiple kilndiff:pass_fn 3
~ 4
test runmultiple kilndiff:pass_fn 0
test runmultiple kilndiff:fail
~ 3
test run kilndiff:optional_fn 2
~ 4
test verify kilndiff:pass_fn
~ 10
test clearall

# test: as a player
execute as Diff0 run test run kilndiff:pass_fn
~ 3
execute as Diff0 run test run kilndiff:fail
~ 3
execute as Diff0 run test runthese
~ 3
execute as Diff0 run test locate kilndiff:*
execute as Diff0 run test pos
execute as Diff0 run test runthat
execute as Diff0 run test clearthat
execute as Diff0 run test resetthat
execute as Diff0 run test create kilndiff:pass_fn
execute as Diff0 run test locate kilndiff:pass_fn
execute as Diff0 run test clearall 50
test clearall

# publish and unpublish are not in a dedicated server's tree
publish
publish true
publish true 25565
publish false 25599
publish survival
publish survival true 25565
unpublish
unpublish now
execute run publish
execute as Diff0 run publish true
execute run unpublish
help publish
help unpublish

# fetchprofile
fetchprofile
fetchprofile name
fetchprofile id
fetchprofile id abc
fetchprofile entity
fetchprofile entity @e
fetchprofile entity Nobody
fetchprofile entity @e[type=minecraft:pig,limit=1]
fetchprofile entity Diff0
fetchprofile entity @a[name=Other0]
fetchprofile entity @a
fetchprofile entity @s
fetchprofile bogus Diff0
fetchprofile name Notch
~ 5
fetchprofile name Diff0
~ 5
fetchprofile name diff0
~ 5
fetchprofile name OTHER0
~ 5
fetchprofile name "Diff 0"
~ 5
fetchprofile name ""
~ 5
fetchprofile name Diff0 extra
~ 5
fetchprofile id 00000000-0000-0000-0000-000000000000
~ 5
fetchprofile id 00000000-0000-0000-0000-000000000001
~ 5
fetchprofile id 1-1-1-1-1
~ 5
fetchprofile id 00000000000000000000000000000001
fetchprofile id @a
execute as Diff0 run fetchprofile entity @s
execute as Diff0 run fetchprofile name Other0
~ 5
execute as Other0 run fetchprofile entity Diff0
execute at Diff0 run fetchprofile entity @p
! scoreboard objectives add fp dummy
execute store success score fp fp run fetchprofile entity @a[limit=1]
execute store result score fp fp run fetchprofile name Diff0
~ 5
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

    def drain(self):
        """The command feedback that arrived since the last command."""
        out = []
        while True:
            try:
                out.append(self.queue.get_nowait())
            except queue.Empty:
                return [l for l in out if not l.endswith("<--[HERE]") and l != UNKNOWN]

    def stop(self):
        if self.p.poll() is None:
            try:
                self.send("stop")
                self.p.wait(timeout=60)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()
        self.log.close()


def reset_lists(base: Path):
    """Both servers keep the whitelist and ban lists next to the world; each run starts empty."""
    for name in ("whitelist.json", "banned-players.json", "banned-ips.json", "usercache.json"):
        (base / name).unlink(missing_ok=True)
def write_zip_pack(dest: Path):
    with zipfile.ZipFile(dest / "kilnzip.zip", "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in ZIP_PACK.items():
            z.writestr(name, text)


def start_vanilla(port: int) -> Server:
    base = SCRATCH / "vanilla"
    base.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(base / "world", ignore_errors=True)
    reset_lists(base)
    # The test functions: a world pack, found and enabled when the world is created.
    shutil.copytree(DATAPACK, base / "world" / "datapacks" / DATAPACK.name)
    write_zip_pack(base / "world" / "datapacks")
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
        f"initial-enabled-packs={INITIAL_PACKS}",
    ]
    (base / "server.properties").write_text("\n".join(props) + "\n", encoding="utf-8")
    argv = ["java", "-Xmx2G", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8", "-Dfile.encoding=UTF-8",
            "-jar", str(WORK / "server.jar"), "--nogui"]
    return Server("vanilla", argv, base, os.environ.copy(), VANILLA_LINE, SCRATCH / "vanilla.log")


def start_kiln(port: int, exe: Path, lang: Path) -> Server:
    base = SCRATCH / "kiln"
    base.mkdir(parents=True, exist_ok=True)
    reset_lists(base)
    copy = SCRATCH / "kiln-diff.exe"
    shutil.copy2(exe, copy)
    env = os.environ.copy()
    env.update({"KILN_PORT": str(port), "KILN_LANG": str(lang), "RUST_LOG": "info", "NO_COLOR": "1"})
    env.pop("KILN_WORLD", None)
    packs = SCRATCH / "kiln-datapacks"
    shutil.rmtree(packs, ignore_errors=True)
    shutil.copytree(DATAPACK, packs / DATAPACK.name)
    write_zip_pack(packs)
    env["KILN_DATAPACKS"] = str(packs)
    env["KILN_INITIAL_PACKS"] = INITIAL_PACKS
    # The vanilla pack (recipes, loot, advancements, the feature packs).
    env.setdefault("KILN_DATAPACK", str(WORK / "generated"))
    env.pop("KILN_OPS", None)
    # Vanilla's offline server asks the session service about every name and id; so does Kiln
    # when told to (its default is to answer offline servers without the network).
    env["KILN_PROFILE_LOOKUP"] = "true"
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
        elif line.startswith("~ "):
            cases.append(("wait", section, line[2:]))
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
    ap.add_argument("-s", dest="sections", action="append", help="only run the sections starting with this text (and the setup lines before the first section)")
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
            if a.sections and section and not any(section.startswith(p) for p in a.sections):
                continue
            if kind == "vanilla":
                run(vanilla, command)
                time.sleep(2)  # let forced chunks load and generate
                continue
            if kind == "setup":
                for server in servers:
                    run(server, command)
                continue
            if kind == "wait":
                time.sleep(float(command))
                want, got = vanilla.drain(), kiln.drain()
                label = f"(console lines after {command} s)"
                ok = want == got
                if ok:
                    passed += 1
                else:
                    failed.append((section, label, want, got))
                if a.verbose or not ok:
                    print(f"{'PASS' if ok else 'FAIL'} [{section}] {label}")
                    for line in want:
                        print(f"    vanilla: {line}")
                    for line in got:
                        print(f"    kiln:    {line}")
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
