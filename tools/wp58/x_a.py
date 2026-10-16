from p68_sulfur import *
prep(); time.sleep(3)
clear_area()
cube = spawn_cube(); eid = cube["id"]
time.sleep(1.0)
give(ME, "minecraft:tnt", 1); give(ME, "minecraft:flint_and_steel", 1)
ok = use_on_entity(a, eid, "minecraft:tnt")
time.sleep(0.8)
ok = use_on_entity(a, eid, "minecraft:flint_and_steel")
F("ignite clicked:", ok, a.state(0)["gameTime"])
for i in range(14):
    time.sleep(1)
    es = ents_of(a, "sulfur_cube")
    F(i, a.state(0)["gameTime"], a.cmd(f"entdata {eid}")[:80] if es else "gone", a.state(0)["health"])
