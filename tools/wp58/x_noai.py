from p68_small import *
sv("kill @e[tag=nai]", show=False)
sv(f"tp {ME} 20.5 200 60.5", show=False)
time.sleep(1.5)
sv("summon minecraft:cow 20.9 200 60.7 {NoAI:1b,Tags:[\"nai\"]}", show=False)
sv("summon minecraft:cow 25.5 200 60.5 {NoAI:1b,Tags:[\"nai\"]}", show=False)
sv("summon minecraft:cow 15.5 200 60.5 {Tags:[\"nai\"]}", show=False)
for i in range(4):
    time.sleep(1)
    o = sv("data get entity @e[type=cow,tag=nai,sort=nearest,limit=1]", show=False, wait=0.3)
    print(i, "overlapping noai", re.findall(r"Motion: \[[^\]]*\]|Pos: \[[^\]]*\]", o))
    o = sv("data get entity @e[type=cow,tag=nai,sort=furthest,limit=1]", show=False, wait=0.3)
    print(i, "far", re.findall(r"Motion: \[[^\]]*\]|Pos: \[[^\]]*\]", o))
sv("kill @e[tag=nai]", show=False)
