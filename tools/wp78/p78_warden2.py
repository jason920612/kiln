"""wp78 item 4c: after the warden is out, a player standing still (resistance V, shrieker triggered once by a jump): pose histogram of the warden at 0.4 s resolution
over DUR seconds, transitions, the player's health and darkness; par.sh on both servers. usage: p78_warden2.py X Y Z"""
import collections, sys
from p72 import *
import p61
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
X, Y, Z = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
DUR = int(os.environ.get("DUR", "90"))
def say(*x): print(*x, flush=True)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False)
sv("kill @e[type=warden]", show=False)
sv(f"gamemode creative {ME}", show=False)
sv(f"execute in minecraft:overworld run tp {ME} {X} {Y + 2} {Z}", show=False)
time.sleep(10)
spot = free_spot(a, X, Y, Z, r=5, dy=3)
sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
sv(f"effect give {ME} minecraft:regeneration 99999 4 true", show=False)
sv(f"execute in minecraft:overworld run tp {ME} {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}", show=False)
sv(f"gamemode survival {ME}", show=False)
time.sleep(2)
# shriek until the warden is out: jumping near the shrieker
t0 = time.time()
while time.time() - t0 < 150 and not [e for e in a.state(60)["entities"] if e["t"] == "warden"]:
    a.key("space", "tap"); time.sleep(0.5)
    a.key("w", "down"); time.sleep(0.3); a.key("w", "up")
tw = time.time()
say("warden out after", round(tw - t0), "s of jumping")
poses = collections.Counter(); trans = []
last = None; dark = []; hp = []; dist = []
t1 = time.time()
while time.time() - t1 < DUR:
    st = a.state(40)
    w = [e for e in st["entities"] if e["t"] == "warden"]
    if not w:
        say("no warden at", round(time.time() - t1)); alive_auto(a); time.sleep(0.5); continue
    p = w[0].get("pose")
    poses[p] += 1
    if p != last:
        trans.append((round(time.time() - t1, 1), p)); last = p
    hp.append(round(st["health"], 1))
    dist.append(round(math.dist((w[0]["x"], w[0]["y"], w[0]["z"]), (st["x"], st["y"], st["z"]))))
    dark.append(any("darkness" in e["id"] for e in st.get("effects", [])))
    time.sleep(0.4)
say("pose samples", dict(poses))
say("transitions", trans[:40])
say("player health samples (every 10th)", hp[::10], "min", min(hp) if hp else None)
say("distance to the warden (every 10th)", dist[::10])
say("darkness shown", sum(dark), "of", len(dark))
