"""wp78 item 5b: the shipwreck at (-384,~,-96): chests opened for real (water breathing and night vision as setup), the treasure map's destination as the client's map shows it,
then dig the buried treasure the map points at (9,~,201 is the nearest on seed 78, checked by locate). par.sh on both servers."""
import collections, hashlib, sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
def say(*x): print(*x, flush=True)
def O(cmd, wait=0.5): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False)
sv(f"gamemode survival {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"effect give {ME} minecraft:water_breathing 3000 0 true", show=False)
sv(f"effect give {ME} minecraft:night_vision 3000 0 true", show=False)
O(f"tp {ME} -384 80 -96")
time.sleep(25)
st = a.state(0)
chests = sorted(map(tuple, a.findbox("chest", -384 - 40, -20, -96 - 40, -384 + 40, 80, -96 + 40, 60)))
say("chests near the shipwreck:", chests)
for pos in chests:
    spot = (0, pos[0], pos[1] + 2, pos[2])
    if not spot:
        say("chest", pos, "no spot"); continue
    O(f"tp {ME} {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}")
    time.sleep(2.5)
    a.close()
    a.use_on(*pos)
    time.sleep(1.5)
    s = a.state(0)
    items = sorted((x["id"].replace("minecraft:", ""), x["n"]) for x in s.get("slots", []) if x["i"] < 27 and x["n"] > 0)
    say("chest", pos, a.screen(), items)
    if any(i[0] == "filled_map" for i in items):
        slot = next(x["i"] for x in s["slots"] if x["id"] == "minecraft:filled_map")
        a.guiclick(slot, "left", shift=True)
        time.sleep(0.5)
    a.close()
inv = a.inv()
say("filled maps in the inventory:", inv.get("minecraft:filled_map", 0))
if inv.get("minecraft:filled_map", 0):
    idx = next(e["slot"] for e in a.state(0)["inv"] if e["id"] == "minecraft:filled_map")
    say("map item:", a.cmd(f"item {idx}")[:400])
    try:
        a.select("minecraft:filled_map")
        time.sleep(1.5)
        say("mapinfo:", a.cmd("mapinfo")[:700])
    except Exception as ex:
        say("mapinfo error", repr(ex)[:100])
