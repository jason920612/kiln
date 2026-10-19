from lock import *
setup(frozen=False)
for t in "KV":
    print(t, gametime(t), ask(t, "locate structure minecraft:monument", wait=8)[-200:])
    print(t, ask(t, "difficulty", wait=0.3)[-100:])
