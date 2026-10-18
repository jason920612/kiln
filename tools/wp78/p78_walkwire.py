from p72 import *
import sys
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
def O(cmd, wait=0.4): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("gamemode creative KilnTest", show=False)
O("forceload add -312 -1240 -288 -1212")
sv("effect give KilnTest minecraft:resistance 99 3 true", show=False)
O("tp KilnTest -302 66 -1230")
time.sleep(12)
O("setblock -302 66 -1226 minecraft:tripwire[attached=true,east=true,west=true]")
sv("gamemode survival KilnTest", show=False)
time.sleep(2)
rows = []
a.key("w", "down")
a.look(0, 10)    # yaw 0 = south (+z)
t0 = time.time()
while time.time() - t0 < 6:
    st = a.state(0)
    wire = a.block(-302, 66, -1226)
    pw = "powered=true" in wire
    d1 = "triggered=true" in a.block(-301, 67, -1219)
    rows.append((round(st["z"], 2), pw, d1))
    time.sleep(0.15)
a.key("w", "up")
print("z / wire powered / dispenser triggered:", rows[::2])
print("arrows:", len([e for e in a.state(40)["entities"] if e["t"] == "arrow"]), "health", a.state(0)["health"])
