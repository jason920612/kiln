from lock import *
import re
NX, NY, NZ = 800, 70, 800
N = "execute in minecraft:the_nether run "
for t in "KV":
    w(t, "gamerule spawn_mobs false"); w(t, "difficulty normal")
    w(t, "gamemode survival KilnTest"); w(t, "effect clear KilnTest")
    w(t, N + f"tp KilnTest {NX + 12}.5 {NY} {NZ + 12}.5")
time.sleep(6)
for t in "KV":
    w(t, "kill @e[type=!player]")
    for i in range(3):
        w(t, N + f'summon minecraft:piglin {NX + 12 + i}.5 {NY} {NZ + 4}.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,Tags:["pg"],HandItems:[{{id:"minecraft:golden_sword",count:1}},{{}}]}}')
for step in range(4):
    for t in "KV": sprint(t, 40)
    for t in "KV":
        w(t, "tick freeze")
    time.sleep(0.4)
    for t in "KV":
        o = ask(t, 'data get entity @e[tag=pg,limit=1] Brain.memories', wait=0.4)
        hp = ask(t, 'data get entity KilnTest Health', wait=0.2)
        pos = ask(t, 'data get entity @e[tag=pg,limit=1] Pos', wait=0.2)
        print((step+1)*40, t, "hp", re.findall(r"-?\d+\.\d+", hp)[-1:], "pos", [round(float(x)) for x in re.findall(r"-?\d+\.\d+", pos)[-3:]], o.strip()[-260:].replace("\n"," "), flush=True)
    for t in "KV": w(t, "tick unfreeze")
