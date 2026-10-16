from p68 import *
import re
def gt():
    out = sv("time query gametime", show=False, wait=0.4)
    m = re.findall(r"(\d+)\]?", out)
    return int(m[-1]) if m else None
def state():
    for h in range(4):
        out = sv("execute if block 100 202 100 minecraft:dried_ghast[hydration=%d]" % h, show=False, wait=0.25)
        if "pass" in out and "fail" not in out:
            return h
    return "gone"
def wait_done():
    time.sleep(0.5)
    for _ in range(200):
        out = sv("tick query", show=False, wait=0.25).lower()
        if "sprinting" not in out:
            return
sv("kill @e[type=happy_ghast]", show=False)
sv("forceload add 100 100 100 100", show=False); time.sleep(1.5)
sv("setblock 100 200 100 minecraft:stone", show=False)
sv("fill 99 201 99 101 204 101 minecraft:air", show=False)
sv("setblock 100 202 100 minecraft:dried_ghast[waterlogged=true,hydration=0]", show=False, wait=0.6)
t0 = gt()
last = 0
for i in range(60):
    sv("tick sprint 500", show=False, wait=0.3)
    wait_done()
    s = state()
    if s != last:
        print("state", s, "at +", gt() - t0, flush=True)
        last = s
    if s == "gone":
        break
print("ghasts:", sv("execute if entity @e[type=happy_ghast]", show=False)[-60:])
