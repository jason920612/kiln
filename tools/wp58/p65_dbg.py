from p65_b2 import *
sv("execute at KilnTest run tp KilnTest 1463.5 111 -1040.5", show=False)
time.sleep(3)
print(sv("execute at KilnTest as @e[type=villager,distance=..70] run data get entity @s Brain.memories", show=False, wait=2.0)[:2500])
print(sv("execute at KilnTest as @e[type=villager,distance=..70] run data get entity @s VillagerData", show=False, wait=2.0)[:800])
