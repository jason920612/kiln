from p68_copper import *
prep(); time.sleep(3)
gid = build()
F("entities near:", [(e["t"], e["id"], round(e["x"],1), round(e["y"],1), round(e["z"],1)) for e in a.state(12)["entities"]])
sv("kill @e[type=!player]", show=False)
reset()
# interact with a NoAI golem to isolate the events
m = summon("copper_golem", "{NoAI:1b}", x=20.5, z=23.5)
eid = m["id"]
for item, label in (("minecraft:honeycomb", "wax"), ("minecraft:iron_axe", "axe-unwax"), ("minecraft:iron_axe", "axe-nothing"), ("minecraft:shears", "shears")):
    give(ME, item, 1)
    rec_start(a)
    ok = use_on_entity(a, eid, item)
    time.sleep(1.0)
    F("golem +", label, ok, [l[:100] for l in lines() if "pickup" not in l])
sv("data merge entity @e[type=copper_golem,limit=1] {next_weather_age:1L}", show=False); time.sleep(1.5)
give(ME, "minecraft:iron_axe", 1)
rec_start(a)
ok = use_on_entity(a, eid, "minecraft:iron_axe")
time.sleep(1.0)
F("axe scrape exposed->unaffected", ok, [l[:100] for l in lines() if "pickup" not in l], sv("data get entity @e[type=copper_golem,limit=1] weather_state", show=False)[-60:])
