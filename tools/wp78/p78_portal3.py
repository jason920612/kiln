from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
X, Y, Z = 1000, 200, 1000
def O(cmd, wait=0.4): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
sv("gamemode creative KilnTest", show=False)
O(f"tp KilnTest {X + 1.5} {Y} {Z - 2.5}")
time.sleep(3)
sv("gamemode survival KilnTest", show=False)
time.sleep(1)
a.cmd("spy on"); a.cmd("spy clear")
a.look(0, 5)
a.key("w", "down")
t0 = time.time()
for i in range(14):
    s = a.state(0)
    print(round(time.time() - t0, 2), round(s["x"], 2), round(s["y"], 2), round(s["z"], 2), flush=True)
    time.sleep(0.12)
a.key("w", "up")
print(a.cmd("spylog")[:1500])
