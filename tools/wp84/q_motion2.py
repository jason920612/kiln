"""player Motion on both servers while the real clients stand, walk, sprint, jump, fall"""
from lab84 import *
cl = clients()
for t in "KV":
    w(t, "gamemode survival KilnTest")
    w(t, "execute in minecraft:overworld run tp KilnTest 20.5 200 10.5")
time.sleep(2.5)
def motion(t):
    m = re.findall(r"-?\d+\.\d+", nbt(t, "entity", "KilnTest", "Motion"))
    return [round(float(x), 4) for x in m[:3]]
def sample(label, n=5, dt=0.25):
    rows = []
    for i in range(n):
        rows.append({t: motion(t) for t in "KV"})
        time.sleep(dt)
    print(label)
    for r in rows:
        print("   K", r["K"], "V", r["V"])
sample("idle")
for c in cl.values():
    c.look(0, 0); c.key("w", "down")
sample("walking", 6)
for c in cl.values():
    c.key("ctrl", "down")
sample("sprinting", 6)
for c in cl.values():
    c.key("space", "down")
sample("sprint-jumping", 8, 0.12)
for c in cl.values():
    c.key("space", "up"); c.key("w", "up"); c.key("ctrl", "up")
sample("after", 4)
for t in "KV":
    w(t, "execute in minecraft:overworld run tp KilnTest 20.5 240 10.5")
sample("falling", 6, 0.2)
