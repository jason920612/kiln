"""wp80 screenshots (on the VM, both servers and both clients up): p80_shots.py eyeblossom|beacon|guardian -> ~/wp80/shots/<scene>_KV.jpg
(the Kiln client's window with the vanilla client's picture in the corner)."""
import os, subprocess, sys
from lab80 import *
scene = sys.argv[1]
os.makedirs(os.path.expanduser("~/wp80/shots"), exist_ok=True)
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false", "do_weather_cycle false"))
both_("tick unfreeze")
revive()
both_("gamemode creative KilnTest")
both_("kill @e[type=!player]")
both_("effect clear KilnTest")
clients = [C(25999), C(26000)]
OB("fill 0 199 0 45 199 45 minecraft:air")


def look(x, y, z):
    for c in clients:
        c.lookat(x, y, z)


if scene == "eyeblossom":
    X, Y, Z = 30, 160, 30
    both_("time set 14000")
    OB(f"fill {X - 9} {Y - 1} {Z - 9} {X + 9} {Y + 4} {Z + 9} minecraft:air")
    OB(f"fill {X - 9} {Y - 1} {Z - 9} {X + 9} {Y - 1} {Z + 9} minecraft:pale_moss_block")
    OB(f"fill {X - 6} {Y} {Z - 6} {X + 6} {Y} {Z + 6} minecraft:closed_eyeblossom")
    for dx in range(-6, 7, 3):
        OB(f"setblock {X + dx} {Y} {Z - 8} minecraft:pale_oak_log")
    both_("gamerule random_tick_speed 600")
    OB(f"tp KilnTest {X}.5 {Y + 1} {Z + 8}.5")
    time.sleep(10)
    both_("gamerule random_tick_speed 3")
    look(X + 0.5, Y + 0.3, Z - 4)
    time.sleep(2)
elif scene == "beacon":
    X, Y, Z = 20, 150, 30
    both_("time set 14500")
    OB(f"fill {X - 12} {Y - 6} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
    for l in range(1, 5):
        OB(f"fill {X - l} {Y - l} {Z - l} {X + l} {Y - l} {Z + l} minecraft:{'diamond_block' if l % 2 else 'iron_block'}")
    OB(f"setblock {X} {Y} {Z} minecraft:beacon{{primary_effect:\"minecraft:speed\",secondary_effect:\"minecraft:regeneration\"}}")
    OB(f"tp KilnTest {X + 9}.5 {Y + 1} {Z - 3}.5")
    time.sleep(14)
    look(X + 0.5, Y + 6, Z + 0.5)
    time.sleep(2)
elif scene == "pale_moss":
    X, Y, Z = 30, 160, 30
    both_("time set 6000")
    OB(f"fill {X - 9} {Y - 1} {Z - 9} {X + 9} {Y + 4} {Z + 9} minecraft:air")
    OB(f"fill {X - 9} {Y - 1} {Z - 9} {X + 9} {Y - 1} {Z + 9} minecraft:grass_block")
    both_("tick freeze")
    time.sleep(0.5)
    time.sleep(1)
    for i, dx in enumerate((-6, -2, 2, 6)):
        kind = "pale_moss_block" if i % 2 == 0 else "moss_block"
        OB(f"setblock {X + dx} {Y - 1} {Z} minecraft:{kind}")
        OB(f"setblock {X + dx - 1} {Y - 1} {Z} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:64}}]}}")
        OB(f"setblock {X + dx - 1} {Y} {Z} minecraft:redstone_block")
    both_("tick unfreeze")
    time.sleep(2)
    # a second round of bone meal for a bigger carpet
    for dx in (-6, -2, 2, 6):
        OB(f"setblock {X + dx - 1} {Y} {Z} minecraft:air")
    time.sleep(1)
    for dx in (-6, -2, 2, 6):
        OB(f"setblock {X + dx - 1} {Y} {Z} minecraft:redstone_block")
    time.sleep(3)
    OB(f"tp KilnTest {X}.5 {Y + 5} {Z + 9}.5")
    look(X + 0.5, Y, Z)
    time.sleep(3)
elif scene == "guardian":
    X, Y, Z = 30, 160, 30
    both_("time set 6000")
    both_("gamemode survival KilnTest")
    both_("attribute KilnTest minecraft:max_health base set 1000")
    both_("effect give KilnTest minecraft:instant_health 1 10 true")
    both_("effect give KilnTest minecraft:water_breathing 999 0 true")
    OB(f"fill {X - 8} {Y - 1} {Z - 8} {X + 8} {Y + 6} {Z + 8} minecraft:prismarine_bricks hollow")
    OB(f"fill {X - 7} {Y} {Z - 7} {X + 7} {Y + 5} {Z + 7} minecraft:water")
    OB(f"tp KilnTest {X - 5}.5 {Y + 1} {Z}.5")
    OB(f"summon minecraft:guardian {X + 5}.5 {Y + 2} {Z}.5 {{PersistenceRequired:1b}}")
    time.sleep(3)
    look(X + 5.5, Y + 2.5, Z + 0.5)
    time.sleep(2.5)
subprocess.run("DISPLAY=:99 import -window root /tmp/wp80shot.png && convert /tmp/wp80shot.png -resize 960x540 -quality 72 ~/wp80/shots/%s_KV.jpg" % scene, shell=True, check=True)
print("saved", scene)
if scene == "guardian":
    both_("kill @e[type=!player]")
