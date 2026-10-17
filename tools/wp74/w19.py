from lock import *
back_to_overworld()
X0, Y0, Z0 = 100, 200, 100
for t in "KV":
    w(t, "gamerule spawn_mobs false"); w(t, "gamerule advance_time false")
    w(t, f"forceload add {X0-20} {Z0-20} {X0+120} {Z0+120}")
    w(t, f"tp KilnTest {X0 + 60} {Y0 + 25} {Z0 + 60}")
time.sleep(10)
for t in "KV":
    w(t, f"fill {X0} {Y0 - 3} {Z0} {X0 + 119} {Y0 + 6} {Z0 + 119} minecraft:air")
    w(t, f"fill {X0} {Y0 - 3} {Z0} {X0 + 119} {Y0 - 1} {Z0 + 119} minecraft:stone")
    w(t, "kill @e[type=!player]")
    w(t, f"fill {X0 + 50} {Y0 - 3} {Z0 + 50} {X0 + 56} {Y0 - 1} {Z0 + 56} minecraft:water")
time.sleep(1)
for t in "KV":
    for i in range(6):
        w(t, f"summon minecraft:turtle {X0 + 52 + (i % 3)}.5 {Y0 - 2} {Z0 + 52 + (i // 3)}.5 {{PersistenceRequired:1b,Tags:[\"tt\"]}}")
    w(t, f"summon minecraft:turtle {X0 + 100}.5 {Y0} {Z0 + 100}.5 {{PersistenceRequired:1b,Tags:[\"tl\"]}}")
time.sleep(1)
for t in "KV":
    print(t, ask(t, 'data get entity @e[tag=tt,limit=1] HomePos')[-90:].strip(), ask(t, 'data get entity @e[tag=tt,limit=1] HasEgg')[-60:].strip())
for step in range(6):
    for t in "KV":
        sprint(t, 200)
    for t in "KV":
        print(t, step, "turtles", count(t, "@e[type=turtle]"), "in water", count(t, f"@e[type=turtle,tag=tt]"), "land one", count(t, "@e[tag=tl]"))
for t in "KV":
    print(t, ask(t, 'data get entity @e[tag=tl,limit=1] Pos')[-90:].strip(), ask(t, 'data get entity @e[tag=tl,limit=1] HomePos')[-90:].strip())
    print(t, ask(t, 'data get entity @e[tag=tt,limit=1] Pos')[-90:].strip())
