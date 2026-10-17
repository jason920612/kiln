from lock import *
for c in ["execute if entity @e[type=villager]", "execute if entity @e[type=villager,tag=steal]", 'execute if entity @e[type=villager,nbt={RestocksToday:0}]', 'execute if entity @e[type=villager,nbt={VillagerData:{level:1}}]',
          'execute as @e[type=villager,limit=1] run data get entity @s VillagerData', 'execute as @e[type=villager,limit=1] run data get entity @s RestocksToday', 'execute as @e[type=villager,limit=1] run data get entity @s Pos']:
    r = ask_both(c)
    print(c, "\n  K", r["K"].strip()[-130:].replace("\n"," | "), "\n  V", r["V"].strip()[-130:].replace("\n"," | "))
