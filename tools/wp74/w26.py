from lock import *
import re
NX, NY, NZ = 800, 70, 800
for t in "KV":
    print(t, "player", ask(t, 'data get entity KilnTest Pos')[-70:].strip(), ask(t, "data get entity KilnTest playerGameType")[-40:].strip(), ask(t, 'data get entity KilnTest active_effects')[-200:].strip().replace("\n"," "))
    w(t, "kill @e[type=!player]")
    w(t, f'execute in minecraft:the_nether run summon minecraft:piglin {NX + 12}.5 {NY} {NZ + 4}.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,Tags:["pp"],HandItems:[{{id:"minecraft:golden_sword",count:1}},{{}}]}}')
time.sleep(0.5)
for step in range(5):
    for t in "KV": sprint(t, 40)
    for t in "KV":
        pos = ask(t, 'data get entity @e[tag=pp,limit=1] Pos', wait=0.2)
        hp = ask(t, 'data get entity KilnTest Health', wait=0.2)
        print((step+1)*40, t, "hp", re.findall(r"-?\d+\.\d+", hp)[-1:], "piglin", [round(float(x)) for x in re.findall(r"-?\d+\.\d+", pos)[-3:]], flush=True)
