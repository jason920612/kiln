"""wp74: four riders on one happy ghast. TARGET=K|V: both real clients (KilnTest agent 25999, Obs 26000) are on that server (switch.sh K|V both); the ghast gets a harness,
KilnTest and Obs are put on it (`ride ... mount`), plus two villagers; the seats as the client sees them, and the ghast flying with the four on board.
usage: TARGET=K|V python3 p74_ghast4.py"""
import os, json, time, re
from lock import *

T_ = os.environ.get("TARGET", "K")
a, b = C(25999), C(26000)
X, Y, Z = 100, 205, 100


def S(c, wait=0.3):
    return ask(T_, c, wait=wait)


S("gamerule spawn_mobs false"); S("gamerule advance_time false")
for n in ("KilnTest", "Obs"):
    if T_ == "V":
        S(f"op {n}")
    S(f"gamemode survival {n}")
    S(f"effect give {n} minecraft:resistance 99999 4 true")
S(f"forceload add {X - 20} {Z - 20} {X + 20} {Z + 20}")
S(f"execute in minecraft:overworld run tp KilnTest {X + 3} {Y} {Z}")
S(f"execute in minecraft:overworld run tp Obs {X - 3} {Y} {Z}")
time.sleep(6)
S(f"fill {X - 15} {Y - 1} {Z - 15} {X + 15} {Y + 20} {Z + 15} minecraft:air")
S(f"fill {X - 15} {Y - 1} {Z - 15} {X + 15} {Y - 1} {Z + 15} minecraft:stone")
S("kill @e[type=!player]")
S(f"execute in minecraft:overworld run tp KilnTest {X + 3} {Y} {Z}")
S(f"execute in minecraft:overworld run tp Obs {X - 3} {Y} {Z}")
S(f'summon minecraft:happy_ghast {X}.5 {Y + 2} {Z}.5 {{PersistenceRequired:1b,Tags:["g"],equipment:{{body:{{id:"minecraft:blue_harness",count:1}}}}}}')
for i in range(2):
    S(f'summon minecraft:villager {X + 2 + i}.5 {Y} {Z + 3}.5 {{PersistenceRequired:1b,Tags:["v{i}"],NoAI:1b}}')
time.sleep(2)
for n in ("KilnTest", "Obs", "@e[tag=v0,limit=1]", "@e[tag=v1,limit=1]"):
    print(T_, "ride", n, S(f"ride {n} mount @e[tag=g,limit=1]", wait=0.5)[-110:].strip().replace("\n", " "), flush=True)
time.sleep(1)
print(T_, "passengers (server):", S("data get entity @e[tag=g,limit=1] Passengers", wait=0.6)[-300:].strip().replace("\n", " "))
for k, c in (("A", a), ("B", b)):
    st = c.state(40)
    g = [e for e in st["entities"] if e["t"] == "happy_ghast"]
    print(T_, k, "client self", round(st["x"], 2), round(st["y"], 2), round(st["z"], 2), "vehicle", st.get("vehicle"), "ghast", [(round(e["x"], 2), round(e["y"], 2), round(e["z"], 2)) for e in g])
    others = [(e["t"], round(e["x"] - g[0]["x"], 2), round(e["y"] - g[0]["y"], 2), round(e["z"] - g[0]["z"], 2)) for e in st["entities"] if e["t"] in ("villager", "player")] if g else []
    print(T_, k, "offsets from the ghast:", others, "| self offset", (round(st["x"] - g[0]["x"], 2), round(st["y"] - g[0]["y"], 2), round(st["z"] - g[0]["z"], 2)) if g else None, flush=True)
# fly: steer with forward key on client A for 6 s (the first rider); the ghast's server position every 0.5 s
a.look(0, 0)
time.sleep(0.5)
a.key("w", "down")
track = []
for i in range(12):
    time.sleep(0.5)
    o = S("data get entity @e[tag=g,limit=1] Pos", wait=0.2)
    nums = [float(x) for x in re.findall(r"-?\d+\.\d+", o)[-3:]]
    track.append([round(v, 1) for v in nums])
a.key("w", "up")
print(T_, "ghast track (every 0.5 s while A holds w):", track)
st = a.state(40)
print(T_, "A after steering: self", round(st["x"], 1), round(st["y"], 1), round(st["z"], 1), "vehicle", st.get("vehicle"), flush=True)
for n in ("KilnTest", "Obs"):
    S(f"ride {n} dismount")
S("kill @e[type=!player]")
