from p65_b2 import *
import p64
from p64 import spylog_ts
p61.VDIR = HOME / "wp65" / "vanilla"
alive(a)
sv("effect clear KilnTest", show=False)
sv("clear KilnTest", show=False)
sv("kill @e[type=#minecraft:raiders]", show=False)
sv("give KilnTest minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier=2] 1", show=False)
sv("execute in minecraft:overworld run tp KilnTest 1463.5 111 -1040.5", show=False)
time.sleep(4)
a.select("minecraft:ominous_bottle")
a.cmd("spy on"); spylog_ts(a)
a.mouse("right", "down")
time.sleep(3.0)
a.mouse("right", "up"); print("inv", a.inv(), "screen", a.screen(), "effects", [e["id"] for e in a.state(0)["effects"]])
for l in spylog_ts(a):
    if "Pos" not in l[:20] and "Move" not in l and "KeepAlive" not in l and "Rot" not in l[:14]:
        print(l[:160])
