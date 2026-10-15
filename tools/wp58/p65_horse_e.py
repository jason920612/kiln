"""wp65: riding a tame saddled horse: pressing E (the inventory key) must open the horse's inventory (PlayerCommand OPEN_INVENTORY)."""
from p65_b2 import *
prep()
sv("fill -10 199 -10 10 199 10 minecraft:grass_block", show=False)
sv("fill -10 200 -10 10 215 10 minecraft:air", show=False)
sv("kill @e[type=!player]", show=False)
sv("tp KilnTest 0.5 200 0.5", show=False); sv("tp Obs 6.5 200 6.5", show=False)
time.sleep(2)
sv("summon minecraft:horse 2.5 200 0.5 {Tame:1b,EatingHaystack:0b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}", show=False)
time.sleep(2)
h = [e for e in a.state(10)["entities"] if e["t"] == "horse"][0]
F("horse seen", h["id"], ent(a, h["id"])["passengers"])
F("mount", interact_ent(a, h["id"], "right", tries=8, dy=1.0), "vehicle", a.state(0)["vehicle"])
time.sleep(1)
a.key("e"); time.sleep(1.5)
F("screen after E while riding:", a.screen())
shot("horse_e")
