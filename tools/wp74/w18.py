from lock import *
for t in "KV":
    w(t, "tick rate 20")
    for d in ("overworld", "the_nether", "the_end"):
        w(t, f"execute in minecraft:{d} run forceload remove all")
    w(t, "execute in minecraft:the_nether run setblock -51 140 -39 air")
    w(t, "execute in minecraft:the_end run setblock 1181 54 -7 air")
    w(t, "setblock -1293 66 735 air")
time.sleep(1)
# does a command block in the nether with a global selector reach the overworld?
for t in "KV":
    w(t, "execute in minecraft:the_nether run forceload add 0 0")
time.sleep(5)
for t in "KV":
    for c in ('execute in minecraft:the_nether run setblock 5 100 5 minecraft:stone', 'execute in minecraft:the_nether run setblock 5 101 5 minecraft:repeating_command_block{auto:1b,TrackOutput:0b,Command:"kill @e[type=cow]"}',
              'execute in minecraft:overworld run summon minecraft:cow 100.5 200 100.5 {PersistenceRequired:1b,NoAI:1b}', 'execute in minecraft:the_nether run summon minecraft:cow 5.5 102 8.5 {PersistenceRequired:1b,NoAI:1b}'):
        w(t, c)
time.sleep(3)
for t in "KV":
    print(t, "cows left", count(t, "@e[type=cow]"), "in overworld", count(t, "@e[type=cow,x=0,y=0,z=0,dx=500,dy=400,dz=500]"))
    w(t, "execute in minecraft:the_nether run setblock 5 101 5 air")
    w(t, "kill @e[type=cow]")
