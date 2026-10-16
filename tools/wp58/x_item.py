from p68_mounts import *
reset()
pool()
sv("summon minecraft:nautilus 34.5 195 33.5 {PersistenceRequired:1b,NoAI:1b}", show=False)
time.sleep(1.0)
for cmd in ("item replace entity @e[type=nautilus,limit=1] armor.body with minecraft:iron_nautilus_armor",
            "data get entity @e[type=nautilus,limit=1] equipment",
            "item replace entity @e[type=nautilus,limit=1] armor.body with minecraft:air",
            "data get entity @e[type=nautilus,limit=1] equipment",
            "item replace entity @e[type=nautilus,limit=1] saddle with minecraft:saddle",
            "data get entity @e[type=nautilus,limit=1] equipment",
            "item replace entity @e[type=nautilus,limit=1] weapon.mainhand with minecraft:stick"):
    print(cmd, "=>", sv(cmd, show=False)[-200:].replace("\n", " | "))
