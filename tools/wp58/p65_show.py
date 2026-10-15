"""wp65: screenshots for the log: a mid-flight elytra shot, the horse screen opened with the inventory key, the raid bar, A seen by B."""
from p65_b2 import *
prep()
alive_b(b); alive(a)
F("B screen", b.screen(), b.state(0)["health"])
for nm in ("KilnTest", "Obs"):
    sv(f"gamemode survival {nm}", show=False)
    sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
sv("clear KilnTest", show=False)
sv("item replace entity KilnTest armor.chest with minecraft:elytra", show=False)
sv("give KilnTest minecraft:firework_rocket[fireworks={flight_duration:3}] 64", show=False)
sv("tp KilnTest 17000.5 330 12000.5", show=False)
time.sleep(4)
a.select("minecraft:firework_rocket")
a.look(-90, 8)
a.key("space", "tap"); time.sleep(0.4); a.key("space", "tap"); time.sleep(0.8)
for i in range(8):
    a.mouse("right", "tap"); time.sleep(0.9)
    if i == 5:
        shot("elytra_flight")
F("elytra flight: A", pos(a), "gliding", a.state(0)["fallFlying"])
