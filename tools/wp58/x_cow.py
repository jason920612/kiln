from p68_small import *
for i in range(3):
    o = sv("data get entity @e[type=cow,tag=p68,limit=1]", show=False, wait=0.5)
    print(re.findall(r"Motion: \[[^\]]*\]|Pos: \[[^\]]*\]|leash: [^,]*,[^,]*,[^,\]]*\]", o))
    o = sv("data get entity @e[type=sulfur_cube,tag=p68,limit=1]", show=False, wait=0.5)
    print("sulfur", re.findall(r"Motion: \[[^\]]*\]|Pos: \[[^\]]*\]", o))
    time.sleep(2)
