"""wp82 item 1: the countdown between waves in lock step. A fresh raid (bad omen) on both servers; when wave 1 exists on both: freeze, kill the raiders,
step 10 ticks at a time and count the ticks until the next wave exists. usage: raidcd82.py   (the bots stand at the village bell, see raid82.py)"""
import threading
from lab import *
VX, VZ = 300, 188

def count(t):
    o = ask(t, "execute if entity @e[type=#minecraft:raiders]", wait=0.15)
    m = re.search(r"pass_count\[(\d+)\]|Count: (\d+)", o)
    return int(m.group(1) or m.group(2)) if m else 0

for t in "KV":
    w(t, "kill @e[type=#minecraft:raiders]"); w(t, "effect clear W0")
    for g in ("raids true", "spawn_mobs false", "advance_time false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "time set day"); w(t, "gamemode creative W0")
    w(t, f"execute in minecraft:overworld run forceload add {VX - 48} {VZ - 48} {VX + 48} {VZ + 48}")
time.sleep(2)
for t in "KV":
    w(t, f"execute in minecraft:overworld run tp W0 {VX}.5 150 {VZ}.5")
time.sleep(25)
for t in "KV":
    w(t, f"execute in minecraft:overworld positioned {VX}.5 0 {VZ}.5 positioned over motion_blocking_no_leaves run tp W0 ~ ~ ~")
time.sleep(3)
for t in "KV":
    w(t, "effect give W0 minecraft:bad_omen 6000 1 true")
t0 = time.time()
while time.time() - t0 < 150 and not (count("K") and count("V")):
    time.sleep(0.3)
F("wave 1 exists after", round(time.time() - t0, 1), "s:", count("K"), count("V"))
time.sleep(4)
both_("tick freeze"); time.sleep(0.5)
F("gametime", gametime("K"), gametime("V"), "raiders", count("K"), count("V"))
both_("kill @e[type=#minecraft:raiders]"); time.sleep(0.5)
step(1)
F("after the kill: raiders", count("K"), count("V"))
done = {}
n = 1
while len(done) < 2 and n < 700:
    step(10); n += 10
    for t in "KV":
        if t not in done and count(t):
            done[t] = n
F("ticks until the next wave exists (10-tick granularity):", done)
both_("tick unfreeze")
