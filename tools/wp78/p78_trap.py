"""wp78 item 5a (repeats): the desert pyramid TNT trap N times on both servers (par.sh). Before the first trial the pyramid is cloned to a backup; between trials it is
cloned back. A survival player (diamond armour, blast protection IV, resistance II) drops onto the pressure plate each time. Prints per trial: the lowest health,
the health 10 s later, the number of items that lay around afterwards, and how many TNT blocks and chests were there."""
import collections, sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
N = int(sys.argv[1]) if len(sys.argv) > 1 else 4
X, Z = -1672, -2292
def say(*x): print(*x, flush=True)
def O(cmd, wait=0.5): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing true", show=False)
sv(f"gamemode creative {ME}", show=False); sv(f"effect give {ME} minecraft:slow_falling 600 0 true", show=False)
O(f"forceload add {X - 20} {Z - 20} {X + 20} {Z + 20}")
O(f"forceload add {X + 130} {Z - 20} {X + 170} {Z + 20}")
O(f"tp {ME} {X} 100 {Z}")
time.sleep(25)
g = a.ground(X, Z, 140)
O(f"fill {X - 30} {g - 1} {Z - 30} {X + 30} {g + 30} {Z + 30} minecraft:air", wait=1.5)
O(f"fill {X - 30} {g - 60} {Z - 30} {X + 30} {g - 2} {Z + 30} minecraft:stone", wait=1.5)
O(f"fill {X - 30} {g - 1} {Z - 30} {X + 30} {g - 1} {Z + 30} minecraft:sand", wait=1.5)
r = O(f"place structure minecraft:desert_pyramid {X} {g - 8} {Z}", wait=6.0)
say("place structure:", r.strip()[-120:])
time.sleep(3)
g = a.ground(X, Z, 140)
plates = a.findbox("stone_pressure_plate", X - 28, g - 45, Z - 28, X + 28, g + 20, Z + 28, 5)
px, py, pz = plates[0]
say("ground", g, "plate", (px, py, pz))
y0, y1 = py - 8, py + 28
mid = (y0 + y1) // 2
dx = 150
def backup():
    O(f"clone {X - 16} {y0} {Z - 16} {X + 16} {mid} {Z + 16} {X + dx - 16} {y0} {Z - 16}", wait=1.5)
    O(f"clone {X - 16} {mid + 1} {Z - 16} {X + 16} {y1} {Z + 16} {X + dx - 16} {mid + 1} {Z - 16}", wait=1.5)
def restore():
    O(f"clone {X + dx - 16} {y0} {Z - 16} {X + dx + 16} {mid} {Z + 16} {X - 16} {y0} {Z - 16}", wait=1.5)
    O(f"clone {X + dx - 16} {mid + 1} {Z - 16} {X + dx + 16} {y1} {Z + 16} {X - 16} {mid + 1} {Z - 16}", wait=1.5)
backup()
say("backup done", O(f"execute if blocks {X - 16} {y0} {Z - 16} {X + 16} {mid} {Z + 16} {X + dx - 16} {y0} {Z - 16} all", wait=1.0)[-60:])
rows = []
for trial in range(N):
    sv("kill @e[type=!player]", show=False)
    sv(f"gamemode creative {ME}", show=False)
    O(f"tp {ME} {X} {g + 5} {Z}")
    restore()
    time.sleep(2)
    sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:slow_falling 3 0 true", show=False)
    for slot, it in (("head", "diamond_helmet"), ("chest", "diamond_chestplate"), ("legs", "diamond_leggings"), ("feet", "diamond_boots")):
        sv(f"item replace entity {ME} armor.{slot} with minecraft:{it}[enchantments={{\"minecraft:blast_protection\":4}}]", show=False)
    sv(f"effect give {ME} minecraft:resistance 99 1 true", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
    tnt0 = len(a.findbox("tnt", X - 28, g - 45, Z - 28, X + 28, g + 20, Z + 28, 50)); ch0 = len(a.findbox("chest", X - 28, g - 45, Z - 28, X + 28, g + 20, Z + 28, 50))
    O(f"tp {ME} {px + 0.5} {py + 1.5} {pz + 0.5}")
    sv(f"gamemode survival {ME}", show=False)
    t0 = time.time(); hp = []
    while time.time() - t0 < 12:
        hp.append(round(a.state(0)["health"], 1)); time.sleep(0.25)
    items = len([e for e in a.state(40)["entities"] if e["t"] == "item"])
    rows.append((trial, tnt0, ch0, min(hp), hp[-1], items))
    say("trial", trial, "tnt", tnt0, "chests", ch0, "lowest health", min(hp), "after 12 s", hp[-1], "items around", items, "min first 4.5 s", min(hp[:18]))
say("lowest health per trial", [r[3] for r in rows], "mean", round(sum(r[3] for r in rows) / len(rows), 2))
