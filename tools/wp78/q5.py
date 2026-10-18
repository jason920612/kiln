from lock import *
respawn_all()
w("V", "gamemode creative KilnTest")
w("V", "execute in minecraft:the_end run tp KilnTest 0 80 8")
w("V", "effect give KilnTest minecraft:slow_falling 99999 0 true")
time.sleep(8)
c = C(26000)
c = C(25999)
s = c.state(150)
import collections
print(s["dimension"], collections.Counter(e["t"] for e in s["entities"]))
for nm in ("end_portal", "dragon_egg", "end_gateway", "bedrock"):
    r = c.findbox(nm, -8, 55, -8, 8, 80, 8, 80)
    print(nm, len(r))
for cmd in ["execute in minecraft:the_end if entity @e[type=ender_dragon]", "data get entity @e[type=ender_dragon,limit=1] Health"]:
    print(cmd, ask("V", cmd, wait=1.0).strip()[-70:])
