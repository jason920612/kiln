"""wp78 item 4b: an ancient city shrieker and a survival player (par.sh): the player jumps near the shrieker; the log shows its warnings, the darkness effect,
the warden's emergence and what it does. Resistance V keeps the player alive so the warden's whole behaviour is seen. usage: p78_warden.py X Y Z (a shrieker with can_summon)"""
import collections, sys
from p72 import *
import p61
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
X, Y, Z = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
DUR = int(os.environ.get("DUR", "240"))

def say(*x): print(*x, flush=True)

if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing true", show=False)
sv(f"gamemode creative {ME}", show=False)
sv(f"execute in minecraft:overworld run tp {ME} {X} {Y + 2} {Z}", show=False)
time.sleep(8)
spot = free_spot(a, X, Y, Z, r=5, dy=3)
say("shrieker", (X, Y, Z), a.block(X, Y, Z)[:100], "standing at", spot)
sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"effect give {ME} minecraft:night_vision 99999 0 true", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
sv(f"give {ME} minecraft:diamond_sword", show=False)
sv(f"execute in minecraft:overworld run tp {ME} {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}", show=False)
sv(f"gamemode survival {ME}", show=False)
time.sleep(3)
t0 = time.time()
last = None
events = []
def nb(path):
    return sv(f"data get block {X} {Y} {Z} {path}", show=False, wait=0.3)
k = 0
while time.time() - t0 < DUR:
    alive_auto(a)
    # jump about near the shrieker (vibrations: jump, step, land)
    a.key("space", "tap")
    time.sleep(0.45)
    a.key("w", "down"); time.sleep(0.35); a.key("w", "up")
    k += 1
    if k % 3:
        continue
    st = a.state(0)
    es = a.state(40)["entities"]
    wardens = [e for e in es if e["t"] == "warden"]
    eff = sorted(e["id"].split(":")[-1] for e in st.get("effects", []))
    blk = a.block(X, Y, Z)
    shr = "shrieking=true" in blk
    row = (round(time.time() - t0), "shrieking" if shr else "-", eff, [(round(w["x"]), round(w["y"]), round(w["z"]), w.get("pose"), round(w["hp"])) for w in wardens], round(st["health"], 1))
    if (row[1:5]) != (last[1:5] if last else None):
        say(*row)
    last = row
    if wardens and not events:
        events.append(round(time.time() - t0))
        say("WARDEN first seen at", events[0], "s")
sv(f"effect clear {ME}", show=False)
say("end; warden data:", sv("data get entity @e[type=warden,limit=1] Brain.memories", show=False, wait=0.6)[-200:])
shot("warden_" + T)
