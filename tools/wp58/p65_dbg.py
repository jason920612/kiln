from p65_b2 import *
sv("kill @e[type=#minecraft:raiders]", show=False)
alive(a); alive(b)
time.sleep(2)
sv("kill @e[type=#minecraft:raiders]", show=False)
print(sv("raid test", show=False)[:200])
print("A", a.screen(), a.state(0)["health"], "B", b.screen())
