"""wp74: screenshot scene (both servers, the two real clients): a glass tank with a turtle and a copper golem (they used to drown in Kiln), a lit campfire with a pig, and powder snow
with a cow and a polar bear; saves ~/wp74/shots/hazards_KV.jpg"""
import os, subprocess, time
from lock import *
X, Y, Z = 40, 210, 40
os.makedirs(os.path.expanduser("~/wp74/shots"), exist_ok=True)
back_to_overworld()
both_("gamerule spawn_mobs false"); both_("gamerule advance_time false"); both_("time set 6000"); both_("weather clear")
both_("gamemode survival KilnTest"); both_("effect clear KilnTest")
both_(f"forceload add {X - 20} {Z - 20} {X + 20} {Z + 20}")
both_(f"effect give KilnTest minecraft:slow_falling 999 0 true")
both_(f"tp KilnTest {X} {Y + 30} {Z - 8}")
for t in "KV":
    wait_loaded(t, X + 14, Y, Z + 8); wait_loaded(t, X - 14, Y, Z - 14)
time.sleep(2)
both_("kill @e[type=!player]")
both_(f"fill {X - 14} {Y - 3} {Z - 14} {X + 14} {Y + 8} {Z + 8} minecraft:air")
both_(f"fill {X - 14} {Y - 1} {Z - 14} {X + 14} {Y - 1} {Z + 8} minecraft:grass_block")
both_(f"tp KilnTest {X} {Y} {Z - 8}")
both_(f"fill {X - 9} {Y} {Z} {X - 5} {Y + 3} {Z + 3} minecraft:glass hollow")
both_(f"fill {X - 8} {Y} {Z + 1} {X - 6} {Y + 2} {Z + 2} minecraft:water")
both_(f"setblock {X - 1} {Y} {Z + 1} minecraft:campfire[lit=true]")
both_(f"fill {X + 4} {Y} {Z + 1} {X + 4} {Y + 1} {Z + 1} minecraft:powder_snow")
both_(f"fill {X + 7} {Y} {Z + 1} {X + 7} {Y + 1} {Z + 1} minecraft:powder_snow")
time.sleep(1)
for n, (e, x) in {"turtle": ("minecraft:turtle", X - 7.5), "golem": ("minecraft:copper_golem", X - 6.5)}.items():
    both_(f"summon {e} {x} {Y + 0.2} {Z + 1.5} {{NoAI:1b,PersistenceRequired:1b}}")
both_(f"summon minecraft:pig {X - 0.5} {Y} {Z + 1.5} {{NoAI:1b,PersistenceRequired:1b}}")
both_(f"summon minecraft:cow {X + 4.5} {Y} {Z + 1.5} {{NoAI:1b,PersistenceRequired:1b}}")
both_(f"summon minecraft:polar_bear {X + 7.5} {Y} {Z + 1.5} {{NoAI:1b,PersistenceRequired:1b}}")
both_(f"effect give KilnTest minecraft:resistance 99999 4 true")
time.sleep(3)
for t in "KV":
    sprint(t, 800)
from ctl import C
for port in (25999, 26000):
    c = C(port)
    c.lookat(X - 1.0, Y + 1.0, Z + 1.5)
time.sleep(2.5)
subprocess.run("DISPLAY=:99 import -window root /tmp/hz.png && convert /tmp/hz.png -resize 960x540 -quality 70 ~/wp74/shots/hazards_KV.jpg", shell=True, check=True)
print("saved")
