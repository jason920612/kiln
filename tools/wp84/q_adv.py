from lab84 import *
for t in "KV":
    w(t, "gamemode survival KilnTest")
    for a_ in ("root", "nuggets", "bread", "pig_kill", "either", "desert"):
        o = ask(t, f"execute if entity @s[advancements={{kw84:{a_}=true}}]", wait=0.3)
        o2 = ask(t, f"execute as KilnTest if entity @s[advancements={{kw84:{a_}=true}}]", wait=0.3)
        print(t, a_, "pass" in o2.lower() or "passed" in o2.lower() or "Test passed" in o2)
    print(t, "xp", nbt(t, "entity", "KilnTest", "XpTotal"), "food", nbt(t, "entity", "KilnTest", "foodLevel"))
