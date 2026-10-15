from p65_b2 import *
alive(a)
sv("effect clear KilnTest", show=False)
sv("clear KilnTest", show=False)
sv("give KilnTest minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier=2] 1", show=False)
sv("execute in minecraft:overworld run tp KilnTest 1463.5 111 -1040.5", show=False)
time.sleep(4)
a.select("minecraft:ominous_bottle")
a.mouse("right", "down")
for i in range(10):
    ef = [e["id"].split(":")[-1] + str(e["amp"]) + "/" + str(e["ticks"]) for e in a.state(0)["effects"]]
    print(i, ef, flush=True)
    time.sleep(0.4)
a.mouse("right", "up")
