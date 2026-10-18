"""wp78 item 5c: buried treasure near (9,~,201): find the chest, dig down to it with a shovel (real mining), open it (real click). par.sh."""
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
def say(*x): print(*x, flush=True)
def O(cmd, wait=0.5): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False)
sv(f"gamemode creative {ME}", show=False); sv(f"effect clear {ME}", show=False); sv(f"clear {ME}", show=False)
sv(f"effect give {ME} minecraft:night_vision 3000 0 true", show=False)
O(f"tp {ME} 9 90 201")
time.sleep(25)
ch = sorted(map(tuple, a.findbox("chest", 9 - 16, 30, 201 - 16, 9 + 16, 120, 201 + 16, 10)))
say("chests", ch)
x, y, z = ch[0]
top = a.ground(x, z, 130)
say("chest", (x, y, z), "surface", top, "column:", [a.block(x, yy, z)[16:40] for yy in range(y + 1, min(y + 6, top + 1))])
sv(f"gamemode survival {ME}", show=False)
sv(f"give {ME} minecraft:diamond_shovel", show=False)
O(f"tp {ME} {x}.5 {top + 1} {z}.5")
time.sleep(3)
a.select("minecraft:diamond_shovel")
for yy in range(top, y, -1):
    b = a.block(x, yy, z)
    if "air" in b.split("[")[0]:
        continue
    ok = a.mine(x, yy, z, timeout=10)
    time.sleep(0.4)
    if not ok:
        say("could not mine", (x, yy, z), b[:50])
        break
    st = a.state(0)
    if st["y"] > yy + 1.2:
        pass
say("dug down; the chest block now:", a.block(x, y, z)[:50], "player y", round(a.state(0)["y"], 1))
a.close()
a.use_on(x, y, z)
time.sleep(1.5)
s = a.state(0)
items = sorted((q["id"].replace("minecraft:", ""), q["n"]) for q in s.get("slots", []) if q["i"] < 27 and q["n"] > 0)
say("chest", (x, y, z), a.screen(), items)
a.close()
