from p68 import *
print(repr(sv("tick query", show=False, wait=0.6)))
print(repr(sv("tick sprint 200", show=False, wait=0.6)))
time.sleep(0.2)
print(repr(sv("tick query", show=False, wait=0.6)))
time.sleep(5)
print(repr(sv("tick query", show=False, wait=0.6)))
