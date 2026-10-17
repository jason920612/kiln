from lock import *
for x,z in ((0,0),(3000,0),(0,3000),(-3000,0),(0,-3000),(3000,3000),(-3000,-3000)):
    r = ask_both(f"execute in minecraft:overworld positioned {x} 64 {z} run locate structure #minecraft:village", wait=6)
    print(x,z,"K",r["K"].strip()[-110:].replace("\n"," | "),"\n   V",r["V"].strip()[-110:].replace("\n"," | "))
