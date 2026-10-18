"""wp78 item 1b: the bastion remnant at nether (-384,~,288) on both servers (par.sh): a creative scout finds chests, gold blocks, spawners; the chests are opened with real clicks
(loot of a structure: same seed, same contents); a mob census while a survival player stands among the piglins; piglin rules once more (gold armour or not)."""
import collections, hashlib
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
def say(*x): print(*x, flush=True)
def N(cmd, wait=0.4): return sv("execute in minecraft:the_nether run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs true", show=False)
sv(f"gamemode creative {ME}", show=False); sv(f"clear {ME}", show=False); sv(f"effect clear {ME}", show=False)
sv(f"effect give {ME} minecraft:fire_resistance 3000 0 true", show=False)
sv(f"effect give {ME} minecraft:slow_falling 3000 0 true", show=False)
N(f"tp {ME} -384 100 288")
time.sleep(30)
R = 40
cen = {}
for nm in ("chest", "gold_block", "blackstone", "gilded_blackstone", "spawner", "lava", "crying_obsidian", "soul_fire", "polished_blackstone_bricks", "nether_gold_ore"):
    rr = sorted(map(tuple, a.findbox(nm, -384 - R, 30, 288 - R, -384 + R, 120, 288 + R, 400)))
    cen[nm] = rr
    say(f"{nm:28s} {len(rr):4d} digest {hashlib.md5(str(rr).encode()).hexdigest()[:8]}", rr[:3])
census = collections.Counter(e["t"] for e in a.state(60)["entities"] if e["t"] != "item")
say("mobs around:", dict(census))
chests = cen["chest"]
for pos in chests[:6]:
    spot = free_spot(a, pos[0], pos[1], pos[2], r=2, dy=2)
    if not spot:
        say("chest", pos, "no spot"); continue
    sv(f"gamemode creative {ME}", show=False)
    N(f"tp {ME} {spot[1] + 0.5} {spot[2]} {spot[3] + 0.5}")
    time.sleep(3)
    a.close()
    a.use_on(*pos)
    time.sleep(1.3)
    s = a.state(0)
    items = sorted((x["id"].replace("minecraft:", ""), x["n"]) for x in s.get("slots", []) if x["i"] < 27 and x["n"] > 0)
    say("chest", pos, a.screen(), items)
    a.close()
