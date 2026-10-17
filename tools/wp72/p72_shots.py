"""wp72: screenshot scene: a lit conduit (frame of prismarine, water) seen by the real Kiln client; saves ~/wp72/shots/conduit_K.jpg"""
import sys, time, subprocess, os
from lock import *
X, Y, Z = 30, 210, 30
os.makedirs(os.path.expanduser("~/wp72/shots"), exist_ok=True)
setup(frozen=False)
both_("gamemode survival KilnTest"); both_("effect clear KilnTest"); both_("kill @e[type=!player]")
both_(f"fill {X - 9} {Y - 9} {Z - 9} {X + 9} {Y + 9} {Z + 9} minecraft:air")
time.sleep(1)
both_(f"fill {X - 8} {Y - 8} {Z - 8} {X + 8} {Y + 8} {Z + 8} minecraft:glass outline")
both_(f"fill {X - 7} {Y - 7} {Z - 7} {X + 7} {Y + 7} {Z + 7} minecraft:water")
both_(f"fill {X - 2} {Y - 2} {Z - 2} {X + 2} {Y + 2} {Z + 2} minecraft:prismarine outline")
both_(f"setblock {X} {Y} {Z} minecraft:conduit[waterlogged=true]")
both_(f"summon minecraft:zombie {X + 5.5} {Y - 1.0} {Z + 0.5} {{NoAI:1b,PersistenceRequired:1b}}")
both_(f"tp KilnTest {X + 0.5} {Y - 3.0} {Z + 6.5}")
time.sleep(1.0)
from ctl import C
for port in (25999, 26000):
    c = C(port)
    c.lookat(X + 0.5, Y + 0.5, Z + 0.5)
time.sleep(14)  # the conduit looks at its frame every 40 ticks; power, particles and the hum follow
for port in (25999, 26000):
    C(port).lookat(X + 0.5, Y + 0.5, Z + 0.5)
time.sleep(2)
subprocess.run("DISPLAY=:99 import -window root /tmp/conduit.png && convert /tmp/conduit.png -resize 960x540 -quality 72 ~/wp72/shots/conduit_KV.jpg", shell=True, check=True)
print("saved")
