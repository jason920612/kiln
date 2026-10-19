"""wp82 item 5: the end gateway's exit search on a built terrain (Kiln against vanilla): the chunk (0,-64) gets a flat end stone floor at y 50 and towers of end stone
(VARIANT picks them); a cow in a gateway at (0,80,-40) with no exit goes to 1024 blocks out along (0,-1). Prints the exit each server saved in the gateway.
usage: gwflat82.py VARIANT   (1: two towers of equal height; 2: plus one taller; 3: the floor only)"""
from lab import *
V_ = int(sys.argv[1]) if len(sys.argv) > 1 else 1
E = "execute in minecraft:the_end run "
for t in "KV":
    w(t, "gamemode creative W0"); w(t, "gamerule spawn_mobs false")
    w(t, E + "forceload add -48 -1070 48 -980")
    w(t, E + "tp W0 0.5 120 -1000.5")
time.sleep(20)
cmds = [
    "fill -48 0 -1070 48 255 -980 minecraft:air",
    "fill 0 50 -1024 15 50 -1009 minecraft:end_stone",
]
if V_ in (1, 2):
    cmds += ["setblock -10 55 -1020 minecraft:end_stone", "setblock -5 55 -1000 minecraft:end_stone", "setblock 12 55 -1004 minecraft:end_stone"]
if V_ == 2:
    cmds += ["setblock 4 60 -1002 minecraft:end_stone"]
if V_ == 4:
    cmds += ["setblock 20 61 -1000 minecraft:end_stone", "setblock -3 58 -1015 minecraft:end_stone"]
for c in cmds:
    both_(E + c)
    time.sleep(0.3)
time.sleep(2)
for t in "KV":
    w(t, E + "tp W0 0.5 100 -30.5")
time.sleep(2)
for t in "KV":
    w(t, E + "setblock 0 80 -40 minecraft:air")
time.sleep(1)
for t in "KV":
    w(t, E + "setblock 0 80 -40 minecraft:end_gateway")
time.sleep(12)
for t in "KV":
    w(t, E + "summon cow 0.5 80 -39.5 {NoAI:1b,PersistenceRequired:1b,Tags:[\"gw\"]}")
time.sleep(8)
for t in "KV":
    F(t, ask(t, E + "data get block 0 80 -40", wait=0.4).strip()[-170:])
