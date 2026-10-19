from lab84 import *
OB("fill 19 200 22 22 202 26 minecraft:air")
OB("setblock 20 200 24 minecraft:decorated_pot[facing=north]")
OB("setblock 20 200 23 minecraft:comparator[facing=south]")
for n in (1, 2, 32, 44, 64):
    OB('data merge block 20 200 24 {item:{id:"minecraft:stone",count:%d}}' % n)
    time.sleep(0.6)
    print(n, {t: (nbt(t, "block", "20 200 23", "OutputSignal"), nbt(t, "block", "20 200 24", "item")[:40]) for t in "KV"})
OB('setblock 20 200 24 minecraft:chest')
OB('data merge block 20 200 24 {Items:[{Slot:0b,id:"minecraft:stone",count:32}]}')
time.sleep(0.6)
print("chest 32/64:", {t: nbt(t, "block", "20 200 23", "OutputSignal") for t in "KV"})
