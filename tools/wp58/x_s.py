from p68_sulfur import *
prep(); time.sleep(3)
eid = fresh_cube()
swallow(eid, "minecraft:stone")
time.sleep(0.5)
give(ME, "minecraft:shears")
ok = use_on_entity(a, eid, "minecraft:shears")
for i in range(4):
    out = sv("execute as @e[type=item] run data get entity @s {}", show=False, wait=0.3)
    F(i, pos(a), re.findall(r"Pos: \[[^\]]*\]|PickupDelay: \d+s?|Motion: \[[^\]]*\]|Age: \d+s", out))
    time.sleep(0.2)
