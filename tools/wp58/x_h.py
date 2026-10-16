from p68_ghast import *
F(pos(a), a.state(0)["health"], a.screen())
for (x,y,z) in ((26,199,22),(21,199,23),(21,200,23),(20,200,20),(10,199,10),(21,201,23)):
    F((x,y,z), sv(f"execute if block {x} {y} {z} minecraft:water", show=False)[-50:].strip(), blk(a, x,y,z))
print(sv("data get entity KilnTest Pos", show=False)[-100:])
