from p68_small import *
import re
def bees():
    o = sv("data get block 6 200 6 bees", show=False, wait=0.5)
    return re.findall(r"ticks_in_hive: \d+", o), re.findall(r"has the following|bees", o)[:1]
print("weather rain ->", sv("weather rain 100000", show=False)[-80:])
sv("setblock 6 200 6 minecraft:bee_nest[facing=south]{bees:[{entity_data:{id:'minecraft:bee',Health:10f},min_ticks_in_hive:1000,ticks_in_hive:50}],flower_pos:[I;3,200,3]}", show=False)
for i in range(4):
    time.sleep(1)
    print(i, bees(), [(e["t"], round(e["x"]), round(e["z"])) for e in a.state(40)["entities"] if e["t"] == "bee"])
sv("weather clear", show=False)
