from p65 import *
a, b = clients()
for p in ("KilnTest","Obs"):
    sv(f"gamemode creative {p}", show=False)
    sv(f"execute in minecraft:the_nether run tp {p} 196.5 80 4.5", show=False)
time.sleep(6)
s=a.state(0); print([round(s[k],1) for k in "xyz"])
sv("execute in minecraft:the_nether run summon minecraft:hoglin 196.5 77 8.5", show=False)
time.sleep(2)
print(sv("execute as @e[type=minecraft:hoglin] run data get entity @s Pos", show=False))
# light levels around: use the client's view of blocks
y=int(s['y'])
print([a.block(int(s['x']),y-1,int(s['z'])+dz) for dz in range(0,3)])
