from p68 import *
print(sv("data get entity Obs Health", show=False, wait=0.5)[-100:])
print(sv("data get entity KilnTest Health", show=False, wait=0.5)[-100:])
print(sv("data get entity Obs DeathTime", show=False, wait=0.5)[-100:])
