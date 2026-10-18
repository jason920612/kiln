from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
X, Y, Z = 1000, 200, 1000
def O(cmd, wait=0.4): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
sv("gamemode creative KilnTest", show=False)
print(a.block(X + 1, Y, Z), a.block(X + 2, Y + 2, Z), a.block(X, Y - 1, Z)[:30])
O(f"tp KilnTest {X + 1.5} {Y} {Z - 2.5}")
time.sleep(3)
sv("gamemode survival KilnTest", show=False)
time.sleep(1)
s = a.state(0); print("start", round(s["x"], 2), round(s["y"], 2), round(s["z"], 2), s["dimension"], "yaw", s["yaw"])
a.look(0, 5)
a.key("w", "down")
for i in range(40):
    s = a.state(0)
    print(round(i * 0.15, 2), round(s["x"], 2), round(s["y"], 2), round(s["z"], 2), s["dimension"].split("/")[-1], flush=True)
    time.sleep(0.15)
    if "overworld" not in s["dimension"]: break
a.key("w", "up")
