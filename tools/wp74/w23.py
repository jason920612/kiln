from lock import *
for t in "KV":
    w(t, "kill @e[type=turtle]")
time.sleep(0.5)
for t in "KV":
    w(t, 'summon minecraft:turtle 105.5 201 105.5 {NoAI:1b,PersistenceRequired:1b,Tags:["z"],has_egg:1b,home_pos:[I;105,200,105]}')
    time.sleep(0.5)
    print(t, ask(t, 'data get entity @e[tag=z,limit=1]', wait=0.5)[-900:].replace("\n"," "))
for t in "KV": w(t, "kill @e[tag=z]")
