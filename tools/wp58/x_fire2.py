from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 5.5 200 5.5", show=False)
time.sleep(1.5)
def floor():
    out = sv("execute as @e[type=item] run data get entity @s Item", show=False, wait=1.0)
    return dict(collections.Counter(re.findall(r'id: "minecraft:(\w+)"', out)))
for how in ("burning then /kill", "on fire from a pig dying in lava"):
    clear_area()
    for i in range(10):
        sv("summon minecraft:cow 30.5 200 30.5 {NoAI:1b,Fire:200s}", show=False, wait=0.05)
    time.sleep(0.5)
    sv("kill @e[type=cow]", show=False, wait=0.5)
    time.sleep(1.0)
    F("10 burning cows killed by /kill:", floor())
    break
clear_area()
for i in range(10):
    sv("summon minecraft:cow 30.5 200 30.5 {NoAI:1b,Fire:200s}", show=False, wait=0.05)
time.sleep(0.3)
sv("data merge entity @e[type=cow,limit=10] {Health:1f}", show=False)
time.sleep(0.2)
F("10 burning cows left to burn:", "cows alive:", len(ents_of(a, "cow", 60)))
time.sleep(4.0)
F("after the fire", floor())
