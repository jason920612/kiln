"""wp81 item 5: the first end gateway's teleport, Kiln (K) against vanilla (V), same world (seed 77). A gateway block without a saved exit is placed on the End's main island at
(GX, 80, GZ) with a cow standing in it; after the cow has gone through (the exit gateway is found, or generated, on the outer islands) read: where the cow stands, and the
gateway block entity at the entry (`exit_portal`, `exact_teleport`, `Age`) and at the exit, and the exit's surroundings.
usage: gateway81.py [GX GZ]    needs both servers (sv.sh) and the bot W0"""
import re, sys, time
from lk import *

GX = int(sys.argv[1]) if len(sys.argv) > 1 else 0
GZ = int(sys.argv[2]) if len(sys.argv) > 2 else 40
GY = 80
E = "execute in minecraft:the_end run "
SEL = "@e[tag=gw,limit=1]"


def cow_pos(t):
    out = ask(t, f"{E}data get entity {SEL} Pos", wait=0.25)
    m = re.search(r"\[(-?[\d.]+)d, (-?[\d.]+)d, (-?[\d.]+)d\]", out)
    return tuple(float(x) for x in m.groups()) if m else None


for t in "KV":
    if t == "V":
        w(t, "op W0")
    w(t, "gamemode creative W0")
    w(t, "gamerule spawn_mobs false")
    w(t, "gamerule advance_time false")
    w(t, f"{E}tp W0 {GX + 0.5} {GY + 25} {GZ - 9.5}")
time.sleep(25)
for t in "KV":
    print(t, "gateway:", ask(t, f"{E}setblock {GX} {GY} {GZ} minecraft:end_gateway", wait=0.3).strip()[-100:])
    print(t, ask(t, f"{E}summon cow {GX + 0.5} {GY} {GZ + 0.5} {{NoAI:1b,PersistenceRequired:1b,Tags:[\"gw\"]}}", wait=0.3).strip()[-100:])
final = {}
for sec in range(0, 60, 3):
    time.sleep(3)
    for t in "KV":
        if t not in final:
            p = cow_pos(t)
            if p and (abs(p[0]) > 300 or abs(p[2]) > 300):
                final[t] = p
    print(sec + 3, "s", {t: final.get(t) for t in "KV"})
    sys.stdout.flush()
    if len(final) == 2:
        break
time.sleep(3)
for t in "KV":
    print(t, "the cow now:", cow_pos(t))
    print(t, "gateway block entity at the entry:", ask(t, f"{E}data get block {GX} {GY} {GZ}", wait=0.3).strip()[-330:])
    p = final.get(t) or cow_pos(t)
    if p:
        # the exit gateway is within a few blocks of the cow (it stands above it): look for the block entity around it
        x, y, z = int(p[0] // 1), int(p[1] // 1), int(p[2] // 1)
        for dy in range(-4, 3):
            for dx in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    o = ask(t, f"{E}execute if block {x + dx} {y + dy} {z + dz} minecraft:end_gateway", wait=0.08)
                    if "pass" in o.lower():
                        print(t, "exit gateway at", (x + dx, y + dy, z + dz), ask(t, f"{E}data get block {x + dx} {y + dy} {z + dz}", wait=0.25).strip()[-300:])
