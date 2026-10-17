from lock import *
import re
back_to_overworld()
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, "tp KilnTest 150 80 150")
time.sleep(8)
cs = clients()
for k, c in cs.items():
    print(k, [ (y, c.block(150, y, 150).replace("Block{minecraft:","").replace("}","")) for y in range(46, 62)])
    print(k, [ (y, c.block(153, y, 143).replace("Block{minecraft:","").replace("}","")) for y in range(50, 58)])
