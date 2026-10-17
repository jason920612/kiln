from lock import *
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, "tp KilnTest -496 100 640"); w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
time.sleep(20)
for t in "KV":
    print(t, "villagers near", count(t, "@e[type=villager,distance=..120]"), "all near", count(t, "@e[type=!player,distance=..120]"), "villagers anywhere", count(t, "@e[type=villager]"))
    print(t, ask(t, "execute if entity @e[type=villager]")[-100:])
cs = clients()
for k, c in cs.items():
    st = c.state(120)
    print(k, "client sees", sorted(set(e["t"] for e in st["entities"])), len([e for e in st["entities"] if e["t"] == "villager"]))
