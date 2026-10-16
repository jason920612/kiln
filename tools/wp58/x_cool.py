from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
sv(f"tp {ME} 20.5 200 20.5", show=False)
sv("clear KilnTest", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
give(ME, "minecraft:diamond_sword", 1)
give(ME, "minecraft:stick", 1)
time.sleep(1.0)
sv("summon minecraft:iron_golem 20.5 200 23.5 {NoAI:1b,PersistenceRequired:1b}", show=False)
time.sleep(1.2)
eid = ents_of(a, "iron_golem", 20)[0]["id"]
def hp():
    return ents_of(a, "iron_golem", 20)[0]["hp"]
a.lookat(20.5, 201.0, 23.5)
for delay in (0.3, 0.6, 1.0, 1.6, 2.5):
    a.select("minecraft:stick")
    time.sleep(2.0)
    h0 = hp()
    a.select("minecraft:diamond_sword")     # the switch resets the attack strength
    time.sleep(delay)
    a.lookat(20.5, 201.0, 23.5)
    a.mouse("left", "tap")
    time.sleep(0.7)
    F("hit", delay, "s after the switch: damage", round(h0 - hp(), 2))
    sv("data merge entity @e[type=iron_golem,limit=1] {Health:100f}", show=False)
    time.sleep(0.5)
