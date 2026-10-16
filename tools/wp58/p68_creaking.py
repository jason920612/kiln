"""wp68 #5: the pale garden: a creaking heart in pale oak logs at night spawns a creaking; it freezes while the player looks at it; hurting it makes the heart
bleed resin; breaking the heart kills it; resin clumps become resin bricks. Kiln (TARGET=K) vs vanilla (TARGET=V). One actor; par.sh."""
from p68_small import *
import sys

scenario("creaking")
H = (20, 202, 24)


def heart_state():
    return re.sub(r"creaking_heart\[|\]", "", blk(a, *H))


def build():
    reset()
    sv("kill @e[type=!player]", show=False)
    sv("fill 12 200 16 30 215 34 minecraft:air", show=False)
    sv("fill 10 199 10 34 199 38 minecraft:grass_block", show=False)
    # the heart between pale oak logs (vertical), with moss around so it can be active
    sv("setblock 20 201 24 minecraft:pale_oak_log[axis=y]", show=False)
    sv("setblock 20 203 24 minecraft:pale_oak_log[axis=y]", show=False)
    sv("setblock 20 202 24 minecraft:creaking_heart[axis=y,natural=true]", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("gamerule spawn_monsters true", show=False)
    sv("difficulty normal", show=False)
    sv("time set midnight", show=False)
    sv("weather clear", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    time.sleep(1.5)


def spawn_watch():
    t0 = time.time()
    states, seen = [], None
    while time.time() - t0 < 60:
        time.sleep(1.0)
        sv("time set midnight", show=False, wait=0.1)
        st = heart_state()
        if not states or states[-1][1] != st:
            states.append((round(time.time() - t0), st))
        cr = ents_of(a, "creaking", 40)
        if cr and seen is None:
            seen = (round(time.time() - t0), [(round(c["x"], 1), round(c["y"], 1), round(c["z"], 1)) for c in cr])
            break
    F("heart states over time:", states)
    F("first creaking at:", seen)
    return seen


def behaviour():
    cr = ents_of(a, "creaking", 40)
    if not cr:
        F("no creaking to watch")
        return
    eid = cr[0]["id"]
    # look away: it moves toward the player; look at it: it freezes
    track = []
    a.look(0, 0)  # south
    for _ in range(14):
        time.sleep(0.5)
        c = [e for e in a.state(40)["entities"] if e["id"] == eid]
        if c:
            track.append((round(c[0]["x"], 1), round(c[0]["z"], 1)))
    F("looking away from it: positions", track)
    track = []
    for _ in range(14):
        c = [e for e in a.state(40)["entities"] if e["id"] == eid]
        if not c:
            break
        a.lookat(c[0]["x"], c[0]["y"] + 1.0, c[0]["z"])
        time.sleep(0.5)
        track.append((round(c[0]["x"], 1), round(c[0]["z"], 1)))
    F("looking at it: positions", track)
    F("creaking nbt:", re.findall(r"Health: [\d.]+f|home_pos: \[[^\]]*\]", sv("data get entity @e[type=creaking,limit=1]", show=False, wait=0.6))[:3])
    # hit it: it takes no damage but the heart bleeds
    a.select("minecraft:diamond_sword") if a.inv().get("minecraft:diamond_sword") else give(ME, "minecraft:diamond_sword", 1)
    a.select("minecraft:diamond_sword")
    rec_start(a)
    hits = 0
    for _ in range(8):
        c = [e for e in a.state(40)["entities"] if e["id"] == eid]
        if not c:
            break
        if math.hypot(c[0]["x"] - pos(a)[0], c[0]["z"] - pos(a)[2]) > 3.0:
            a.walk_to(c[0]["x"], c[0]["z"], timeout=2, tol=2.2)
        a.lookat(c[0]["x"], c[0]["y"] + 1.0, c[0]["z"])
        time.sleep(0.3)
        a.mouse("left", "tap")
        hits += 1
        time.sleep(0.8)
    ls = json.loads(a.cmd("spylog clear"))
    F("hits", hits, "sounds/particles:", dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:60] for l in ls if "sound" in l or "particle" in l).most_common(8)))
    F("blocks around the heart:", sorted(set(re.sub(r"\[.*", "", blk(a, 20 + dx, 202 + dy, 24 + dz)) for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1))))
    F("creaking health after:", re.findall(r"Health: [\d.]+f", sv("data get entity @e[type=creaking,limit=1]", show=False, wait=0.6)), "heart:", heart_state())
    # break the heart: the creaking dies
    sv("setblock 20 202 24 minecraft:air destroy", show=False)
    time.sleep(2)
    F("after breaking the heart: creakings", len(ents_of(a, "creaking", 40)), "items", dict(collections.Counter(e.get("item") for e in a.state(12)["entities"] if e["t"] == "item")))


def resin():
    reset()
    sv("kill @e[type=!player]", show=False)
    give(ME, "minecraft:resin_clump", 9)
    give(ME, "minecraft:resin_block", 1)
    sv(f"clear {ME} minecraft:diamond_sword", show=False)
    # a clump on a wall (resin clumps are multi-face blocks)
    sv("fill 14 200 14 26 203 26 minecraft:air", show=False)
    sv("setblock 20 200 22 minecraft:stone", show=False)
    time.sleep(0.5)
    a.select("minecraft:resin_clump")
    a.lookat(20.5, 200.5, 21.99)
    time.sleep(0.5)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("resin clump placed on the wall:", blk(a, 20, 200, 21), [l[:70] for l in json.loads(a.cmd("spylog clear")) if "sound" in l][:2])
    # craft resin bricks from 9? (4 resin bricks from 1 resin block recipe: bricks = 4 clumps -> 4 bricks)  use smelting recipe: resin clump -> resin brick
    sv("setblock 18 200 20 minecraft:furnace[facing=south]", show=False)
    sv("item replace block 18 200 20 container.0 with minecraft:resin_clump 3", show=False)
    sv("item replace block 18 200 20 container.1 with minecraft:coal 1", show=False)
    time.sleep(1)
    sv("tick sprint 400", show=False, wait=0.5)
    for _ in range(60):
        if "sprinting" not in sv("tick query", show=False, wait=0.3).lower():
            break
        time.sleep(1)
    F("furnace after smelting resin:", re.findall(r'count: \d+, Slot: \d+b, id: "minecraft:\w+"', sv("data get block 18 200 20 Items", show=False, wait=0.6)))


if __name__ == "__main__":
    prep()
    time.sleep(2)
    for which in sys.argv[1:] or ["main", "resin"]:
        try:
            if which == "main":
                build()
                if spawn_watch():
                    behaviour()
            else:
                resin()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
    sv("gamerule spawn_mobs false", show=False)
    sv("time set day", show=False)
