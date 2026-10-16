from p68_small import *
sv("gamerule random_tick_speed 4096", show=False)
for i, b in enumerate(("copper_chest[facing=south]", "copper_golem_statue[facing=south]", "lightning_rod[facing=up]", "copper_bulb", "copper_door[half=lower]", "copper_trapdoor", "copper_lantern", "chiseled_copper", "copper_bars", "copper_chain[axis=y]", "copper_torch")):
    sv(f"setblock {4+2*i} 200 50 minecraft:{b}", show=False)
time.sleep(25)
for i in range(11):
    print(blk(a, 4 + 2 * i, 200, 50))
sv("gamerule random_tick_speed 3", show=False)
