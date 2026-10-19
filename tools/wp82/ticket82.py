"""wp82 item 5: a portal ticket across a restart, side by side. phase A: a baby cow goes through a nether portal (ticket for 300 ticks); the age it has 5 s after arriving is
printed and the script exits (stop and restart both servers); phase B: the age of the cow in the nether every 2 s for 20 s (the nether chunk keeps ticking for the rest of the
ticket if it was saved, else it is not loaded and the cow is not found). usage: ticket82.py A|B"""
import os
from lab import *
PH = sys.argv[1]
X, Y, Z = 160, 100, 160

def age(t):
    o = ask(t, "execute as @e[tag=tk82] at @s if dimension minecraft:the_nether run data get entity @s Age", wait=0.3)
    m = re.search(r"(-?\d+)\b[^\d]*$", o.strip().splitlines()[-1]) if o.strip() else None
    return int(m.group(1)) if m and "Age" not in o[-6:] and "entity" not in o.splitlines()[-1][-12:] else None

if PH == "A":
    setup(frozen=False, size=60)
    for t in "KV":
        w(t, "execute in minecraft:the_nether run forceload remove all")
        w(t, "kill @e[tag=tk82]")
        w(t, "gamemode creative W0"); w(t, "execute in minecraft:overworld run tp W0 300 120 300")
    time.sleep(5)
    ex = "execute in minecraft:overworld run "
    both_(ex + f"fill {X - 3} {Y - 2} {Z - 3} {X + 6} {Y + 8} {Z + 3} minecraft:air")
    both_(ex + f"fill {X - 2} {Y - 1} {Z - 2} {X + 5} {Y - 1} {Z + 2} minecraft:stone")
    both_(ex + f"fill {X} {Y} {Z} {X + 3} {Y + 4} {Z} minecraft:obsidian")
    both_(ex + f"fill {X + 1} {Y + 1} {Z} {X + 2} {Y + 3} {Z} minecraft:nether_portal[axis=x]")
    time.sleep(1)
    both_(ex + f"summon minecraft:cow {X + 1}.5 {Y + 1} {Z}.5 {{Tags:[\"tk82\"],NoAI:1b,PersistenceRequired:1b,Age:-24000}}")
    time.sleep(2.5)
    for t in "KV":
        F(t, "age 5 s after the trip:", ask(t, "execute as @e[tag=tk82] at @s if dimension minecraft:the_nether run data get entity @s Age", wait=0.4).strip()[-60:])
else:
    t0 = time.time()
    while time.time() - t0 < 30:
        row = {}
        for t in "KV":
            o = ask(t, "execute as @e[tag=tk82] at @s if dimension minecraft:the_nether run data get entity @s Age", wait=0.4).strip()
            row[t] = o.splitlines()[-1][-40:] if o else None
        F(round(time.time() - t0, 1), row)
        time.sleep(0.6)
