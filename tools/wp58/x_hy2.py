from p68 import *
sv("kill @e[type=happy_ghast]", show=False)
sv("forceload add 100 100 100 100", show=False); time.sleep(1.5)
sv("setblock 100 200 100 minecraft:stone", show=False)
sv("fill 99 201 99 101 204 101 minecraft:air", show=False)
sv("gamerule random_tick_speed 3000", show=False)
sv("setblock 100 202 100 minecraft:dried_ghast[waterlogged=true,hydration=3]", show=False, wait=0.6)
for i in range(12):
    sv("tick sprint 600", show=False, wait=1.0)
    time.sleep(1.5)
    out = sv("execute if entity @e[type=happy_ghast]", show=False)
    blk_ = sv("execute if block 100 202 100 minecraft:dried_ghast", show=False)
    print(i, "ghast:", out[-45:].strip(), "| block:", blk_[-30:].strip(), flush=True)
    if "pass" in out and "fail" not in out:
        break
print(sv("data get entity @e[type=happy_ghast,limit=1]", show=False)[-900:])
sv("gamerule random_tick_speed 3", show=False)
