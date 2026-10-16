"""wp68 #5: the crafter with a real lever: fill its 3x3 grid in its screen, switch a slot off, pull the lever, see what it ejects; Kiln vs vanilla."""
from p68_small import *
import sys

scenario("crafter")
CRAFTER = (20, 200, 25)


def run():
    prep()
    time.sleep(2)
    if a.screen():
        a.key("esc", "tap")
        time.sleep(0.6)
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 21.5", show=False)
    time.sleep(1.0)
    sv("clear KilnTest", show=False)
    sv("setblock 20 200 25 minecraft:crafter[orientation=south_up]", show=False)
    sv("setblock 19 200 25 minecraft:lever[face=wall,facing=west,powered=false]", show=False)
    time.sleep(0.5)
    F("blocks:", blk(a, 20, 200, 25), blk(a, 19, 200, 25))
    give(ME, "minecraft:oak_planks", 16)
    give(ME, "minecraft:stick", 8)
    give(ME, "minecraft:iron_ingot", 9)
    # open the crafter
    a.lookat(20.5, 200.6, 25.5)
    time.sleep(0.6)
    a.mouse("right", "tap")
    time.sleep(1.2)
    F("screen:", a.screen(), a.state(0).get("menu"), a.state(0).get("title"))
    st = a.state(0)
    slots = st["slots"]
    F("slot count", len(slots), "first slots", [(s["i"], s["id"]) for s in slots[:12]])
    planks = [s["i"] for s in slots if s["id"] == "minecraft:oak_planks"]
    sticks = [s["i"] for s in slots if s["id"] == "minecraft:stick"]
    F("planks slot", planks, "sticks slot", sticks)
    # wooden pickaxe: planks across the top, sticks down the middle column
    p, s_ = planks[0], sticks[0]
    for g in (0, 1, 2):
        a.guiclick(p, "left")
        a.guiclick(g, "right")
        a.guiclick(p, "left")
    for g in (4, 7):
        a.guiclick(s_, "left")
        a.guiclick(g, "right")
        a.guiclick(s_, "left")
    time.sleep(0.8)
    st = a.state(0)
    F("grid:", [(x["i"], x["id"].split(":")[-1], x["n"]) for x in st["slots"][:9]], "result slot 45?:", [(x["i"], x["id"]) for x in st["slots"][9:10]])
    # switch a slot off: click the empty corner slot 8 with an empty cursor (the crafter's slot button)
    rec_start(a)
    a.guiclick(8, "left")
    time.sleep(0.8)
    F("after clicking the empty slot 8:", [(x["i"], x["id"].split(":")[-1], x["n"]) for x in a.state(0)["slots"][:10]], [l[:80] for l in lines() if "Container" in l or "sound" in l][:5])
    a.key("esc", "tap")
    time.sleep(0.8)
    F("crafter nbt:", sv("data get block 20 200 25", show=False)[-500:])
    # pull the lever from the player's side
    sv(f"tp {ME} 18.5 200 25.5", show=False)
    time.sleep(1.0)
    a.lookat(19.5, 200.5, 25.5)
    time.sleep(0.6)
    F("aim at the lever:", a.state(0).get("hit"))
    rec_start(a)
    seen = set()
    items = collections.Counter()
    a.mouse("right", "tap")
    t0 = time.time()
    while time.time() - t0 < 5:
        time.sleep(0.2)
        for e in a.state(10)["entities"]:
            if e["t"] == "item" and e["id"] not in seen:
                seen.add(e["id"])
                items[e.get("item")] += e.get("n", 1)
    ls = lines()
    F("lever pulled:", blk(a, 19, 200, 25), "crafter:", blk(a, 20, 200, 25), "ejected:", dict(items), "events:", dict(collections.Counter(re.sub(r"vol=\S+", "", l)[:70] for l in ls if "sound" in l or "level_event" in l or "particle" in l or "block_event" in l)))
    F("crafter nbt after:", sv("data get block 20 200 25", show=False)[-500:])


if __name__ == "__main__":
    try:
        run()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
