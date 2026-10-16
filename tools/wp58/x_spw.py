from p68_small import *
sv(f"tp {ME} 20.5 200 30.5", show=False)
time.sleep(1.5)
sv("setblock 22 200 30 minecraft:spawner{SpawnData:{entity:{id:'minecraft:zombie'}},Delay:100s,MaxNearbyEntities:6s}", show=False)
for d in (0.0, 0.5, 2.0):
    time.sleep(d)
    print(d, re.sub(r".*(block data: |query\[)", "", sv("data get block 22 200 30", show=False, wait=0.4))[:420])
sv("setblock 22 200 30 minecraft:air", show=False)
