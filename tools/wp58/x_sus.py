from p68_brush import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
print(sv('setblock 20 199 23 minecraft:suspicious_sand{LootTable:"minecraft:archaeology/desert_pyramid",LootTableSeed:11L}', show=False)[-200:])
time.sleep(0.5)
print(blk(a, 20, 199, 23), blk(a, 20, 199, 22))
print(sv("data get block 20 199 23", show=False)[-300:])
