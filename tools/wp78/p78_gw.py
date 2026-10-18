from p72 import *
import sys
a = C(25999)
sv("gamemode creative KilnTest", show=False)
sv("execute in minecraft:the_end run tp KilnTest 0 90 0", show=False)
time.sleep(6)
r = a.findbox("end_gateway", -110, 50, -110, 110, 120, 110, 40)
print(T, "gateways:", sorted(map(tuple, r)))
b = a.findbox("bedrock", -110, 50, -110, 110, 120, 110, 400)
print(T, "bedrock around gateways (count):", len(b))
