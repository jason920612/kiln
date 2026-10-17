from lock import *
back_to_overworld()
for t in "KV":
    w(t, "execute in minecraft:overworld run tp KilnTest -496 100 640")
time.sleep(25)
cs = clients()
for k, c in cs.items():
    st = c.state(100)
    print(k, round(st["x"]), round(st["y"]), round(st["z"]), "entities", sorted(set(e["t"] for e in st["entities"])), "bells", c.find("bell", 60, 3), "blk", c.block(-496, 68, 640))
for t in "KV":
    print(t, "villagers", count(t, "@e[type=villager]"), "cats", count(t, "@e[type=cat]"), "golems", count(t, "@e[type=iron_golem]"), "all", count(t, "@e[type=!player]"))
