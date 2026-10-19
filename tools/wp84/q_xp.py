from lab84 import *
def xp(t): return nbt(t, "entity", "KilnTest", "XpTotal")
for t in "KV":
    w(t, "clear KilnTest"); w(t, "advancement revoke KilnTest everything"); w(t, "experience set KilnTest 0 points")
time.sleep(1.5)
print("start", {t: xp(t) for t in "KV"})
for t in "KV":
    w(t, "give KilnTest minecraft:gold_nugget 3")
time.sleep(2.5)
print("after nuggets", {t: xp(t) for t in "KV"})
for t in "KV":
    w(t, "give KilnTest minecraft:stick 1")
time.sleep(2.0)
print("after stick", {t: xp(t) for t in "KV"})
for t in "KV":
    w(t, "kill @e[type=experience_orb]")
