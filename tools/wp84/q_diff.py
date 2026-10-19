from lab84 import *
for t in "KV":
    print(t, repr(ask(t, "difficulty", wait=0.4))[-120:])
