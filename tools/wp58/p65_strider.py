"""wp65: ride a saddled strider across a lava channel with warped fungus on a stick (both clients; B watches), Kiln vs vanilla (TARGET)."""
from p65_b2 import *

def strider():
    prep()
    if T == "V":
        pass
    dimx = "minecraft:the_nether"
    def nsv(c):
        return sv("execute in %s run %s" % (dimx, c), show=False)
    nsv("forceload add 590 590 660 640")
    time.sleep(3)
    for i in range(20):
        out = nsv("setblock 600 99 600 minecraft:netherrack")
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    nsv("fill 590 92 596 660 99 616 minecraft:netherrack")
    nsv("fill 590 100 596 660 112 616 minecraft:air")
    nsv("fill 600 95 604 650 99 608 minecraft:lava")
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"give {nm} minecraft:saddle 1", show=False)
        sv(f"give {nm} minecraft:warped_fungus_on_a_stick 1", show=False)
        sv(f"execute in {dimx} run tp {nm} 601.5 100 603.3", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("kill @e[type=minecraft:strider]", show=False)
    nsv("summon minecraft:strider 601.5 99.9 605.0")
    time.sleep(4)
    st = [e for e in a.state(15)["entities"] if e["t"] == "strider"]
    F("strider seen", len(st), st[0]["hp"] if st else None)
    sid = st[0]["id"]
    # saddle it (right click with the saddle), then mount (right click)
    a.select("minecraft:saddle")
    F("saddle click", interact_ent(a, sid, "right", tries=8, dy=0.9))
    F("saddled?", ent(a, sid)["passengers"], "A vehicle", a.state(0)["vehicle"])
    F("entdata", a.cmd("entdata %d" % sid))
    if a.state(0)["vehicle"] is None:
        F("mount click", interact_ent(a, sid, "right", tries=8, dy=0.9))
    F("A vehicle after", a.state(0)["vehicle"])
    a.select("minecraft:warped_fungus_on_a_stick")
    time.sleep(1)
    # idle ride: the strider does not move on its own while ridden without the stick? measure 3 s with the stick held, no boost
    p0 = pos(a)
    a.key("w", "down"); time.sleep(3); a.key("w", "up")
    p1 = pos(a)
    F("ridden 3 s with w, no boost: moved", round(math.hypot(p1[0] - p0[0], p1[2] - p0[2]), 2), "y", p1[1])
    # boost: right-click with the stick
    a.mouse("right", "tap"); time.sleep(0.2)
    p2 = pos(a)
    a.key("w", "down"); time.sleep(3); a.key("w", "up")
    p3 = pos(a)
    F("3 s after a boost click: moved", round(math.hypot(p3[0] - p2[0], p3[2] - p2[2]), 2))
    e = ent(a, sid); F("strider seen by A: y", round(e["y"], 2), "passengers", e["passengers"])
    eb = ent(b, sid); F("strider seen by B: passengers", eb["passengers"], "pose", eb["pose"])
    shot("strider")
    sv("effect clear KilnTest", show=False)

if __name__ == "__main__":
    strider()
