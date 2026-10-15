from p65 import *
a, b = clients()
print(sv("tp KilnTest 0.5 235 0.5", show=False))
time.sleep(1.5)
print(a.state(0)["y"], a.screen(), a.state(0)["health"])
print(sv("data get entity KilnTest Pos", show=False), sv("data get entity KilnTest Health", show=False))
