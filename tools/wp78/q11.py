from lock import *
import re
n = logpos("K")
w("K", "execute in minecraft:overworld run forceload add 6000 6000 6143 6143")
time.sleep(70)
txt = since("K", n)
for l in txt.splitlines():
    if "mspt mean" in l or "forceload" in l:
        print(re.sub(r"^\S+ +INFO +", "", l)[:230])
w("K", "execute in minecraft:overworld run forceload remove all")
