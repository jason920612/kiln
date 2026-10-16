from p68_small import *
sv("setblock 5 200 35 minecraft:comparator[facing=south]", show=False)
time.sleep(2)
s = ""
for _ in range(30):
    s += "1" if "pass" in sv("execute if block 5 200 34 minecraft:redstone_lamp[lit=true]", show=False, wait=0.2) else "0"
print(s)
