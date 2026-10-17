from lock import *
t="V"
print(ask(t, "execute in minecraft:overworld run setblock 105 200 105 minecraft:stone")[-100:])
print(ask(t, "execute in minecraft:overworld run setblock 105 195 105 minecraft:stone")[-100:])
print(ask(t, 'summon minecraft:villager 105.5 201 105.5 {PersistenceRequired:1b,Tags:["zz"]}')[-100:])
print(ask(t, 'execute if entity @e[tag=zz]')[-60:])
time.sleep(2)
print(ask(t, 'execute if entity @e[tag=zz]')[-60:])
print(ask(t, 'summon minecraft:cow 105.5 201 106.5 {PersistenceRequired:1b,Tags:["zz2"]}')[-100:])
time.sleep(1)
print(ask(t, 'execute if entity @e[tag=zz2]')[-60:])
print(ask(t, 'forceload query')[-200:])
print(ask(t, 'data get entity KilnTest Pos')[-100:], ask(t, 'data get entity KilnTest Dimension')[-100:])
