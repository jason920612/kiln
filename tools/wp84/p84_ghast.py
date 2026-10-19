"""wp84 #1: the happy ghast, one actor against each server at once (par.sh): a ghastling that follows a player holding snowballs (and does not follow one holding a stick),
snowball feeding (Age after each real click), an adult that stays still while the player stands on it, harness by real click and removal with shears, a lead
(fence post, then led by the player). Facts per target in ~/wp84/facts/ghast84_<K|V>.txt, packets in ~/wp84/spy/.
usage: par.sh p84_ghast.py [phases: follow,feed,stand,harness,lead]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
PH = (sys.argv[1] if len(sys.argv) > 1 else "follow,feed,stand,harness,lead").split(",")
ME = "KilnTest"
scenario("ghast84")


def ghasts(c=None, rng=60):
    return [e for e in (c or a).state(rng)["entities"] if e["t"] == "happy_ghast"]


def gpos():
    g = ghasts()
    return (round(g[0]["x"], 2), round(g[0]["y"], 2), round(g[0]["z"], 2)) if g else None


def sget(path):
    return nbt_of("@e[type=happy_ghast,limit=1]", path, 0.5)


def reset(x=20.5, z=20.5):
    kill_all()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"tp {ME} {x} 200 {z}", show=False)
    time.sleep(1.5)


def hyp(p, q):
    return round(math.hypot(p[0] - q[0], p[2] - q[2]), 1)


def follow():
    """The ghastling 8 blocks away; the player holds snowballs for 6 s standing, then walks 10 blocks away and waits 10 s: distance every 2 s. Then a stick."""
    reset()
    give(ME, "minecraft:snowball", 16)
    give(ME, "minecraft:stick", 1)
    sv("summon minecraft:happy_ghast 20.5 203 29.5 {Age:-24000,PersistenceRequired:1b}", show=False)
    time.sleep(1.0)
    a.select("minecraft:snowball")
    for label, item, ticks in (("snowball", "minecraft:snowball", 5), ("stick", "minecraft:stick", 4)):
        a.select(item)
        series = []
        a.look(0, 5)
        for i in range(ticks):
            time.sleep(2.0)
            series.append(hyp(pos(a), gpos()) if gpos() else None)
        F("follow", label, "standing: distance to the ghastling every 2 s", series)
        # walk 10 blocks towards -x and wait
        a.walk_to(8.5, 20.5, timeout=12, tol=1.0)
        series = []
        for i in range(ticks + 1):
            time.sleep(2.0)
            series.append(hyp(pos(a), gpos()) if gpos() else None)
        F("follow", label, "after walking 12 blocks: distance every 2 s", series, "ghastling", gpos(), "player", pos(a))
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        sv("tp @e[type=happy_ghast] 20.5 203 29.5", show=False)
        time.sleep(1.0)
    F("follow done: baby=", sget("Age"))


def feed():
    reset()
    give(ME, "minecraft:snowball", 16)
    sv("summon minecraft:happy_ghast 20.5 202 24.5 {Age:-24000,NoAI:1b,PersistenceRequired:1b}", show=False)
    time.sleep(1.2)
    g = ghasts()[0]
    eid = g["id"]
    F("ghastling client:", {k: g.get(k) for k in ("pose", "bbh", "bbw", "baby")}, "Age", sget("Age"), "ForcedAge", sget("ForcedAge"))
    for i in range(4):
        rec_start(a)
        ok = use_on_entity(a, eid, "minecraft:snowball", dy=0.5)
        time.sleep(0.8)
        cnt = rec_stop(a, f"feed{i}", show=False)
        fx = sorted(k for k in cnt if k.startswith(("sound", "particle", "entity_event")))
        F("feed", i, "click", ok, "Age", sget("Age"), "snowballs left", a.inv().get("minecraft:snowball", 0), "fx", fx[:6])
    # an adult refuses the snowball (no feeding): Age 0
    sv("data merge entity @e[type=happy_ghast,limit=1] {Age:0}", show=False)
    time.sleep(0.6)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:snowball", dy=1.2)
    time.sleep(0.8)
    cnt = rec_stop(a, "adult_snowball", show=False)
    F("adult + snowball: click", ok, "snowballs left", a.inv().get("minecraft:snowball", 0), "Age", sget("Age"), "fx", sorted(k for k in cnt if k.startswith(("sound", "particle", "entity_event")))[:6])


def stand():
    """An adult with the harness and AI; the player is put on its back (tp) and watches: ghast displacement over 10 s with the player on it, then 10 s without."""
    reset()
    sv(f'summon minecraft:happy_ghast 20.5 204 20.5 {{PersistenceRequired:1b,equipment:{{body:{{id:"minecraft:blue_harness",count:1}}}}}}', show=False)
    time.sleep(1.5)
    g = ghasts()[0]
    F("adult:", {k: g.get(k) for k in ("pose", "bbh", "bbw", "baby")})
    sv(f"tp {ME} {g['x']:.2f} {g['y'] + g['bbh'] + 0.2:.2f} {g['z']:.2f}", show=False)
    time.sleep(2.0)
    p0 = gpos()
    track = []
    for i in range(10):
        time.sleep(1.0)
        s = a.state(0)
        gp = gpos()
        track.append((round(s["y"] - gp[1], 2), s.get("onGround"), round(hyp(gp, p0), 2)))
    F("player on top, ghast horizontal displacement from start each second (y rel, onGround, d):", track)
    F("server:", sget("Pos"), "player", nbt_of(ME, "Pos", 0.4))
    sv(f"tp {ME} 30.5 200 30.5", show=False)
    time.sleep(1.0)
    p0 = gpos()
    track = []
    for i in range(10):
        time.sleep(1.0)
        track.append(round(hyp(gpos(), p0), 2))
    F("nobody on top, ghast displacement each second:", track)


def harness():
    reset()
    sv("summon minecraft:happy_ghast 20.5 202 24.5 {NoAI:1b,PersistenceRequired:1b}", show=False)
    time.sleep(1.2)
    eid = ghasts()[0]["id"]
    sv(f"give {ME} minecraft:shears 1", show=False)
    for col in ("white", "red", "lime"):
        give(ME, f"minecraft:{col}_harness", 1)
    for col in ("red", "lime"):
        rec_start(a)
        ok = use_on_entity(a, eid, f"minecraft:{col}_harness", dy=1.5)
        time.sleep(0.8)
        cnt = rec_stop(a, f"harness_{col}", show=False)
        F("harness", col, "click", ok, "body", sget("equipment"), "inv", {k: v for k, v in a.inv().items() if "harness" in k}, "fx", sorted(k for k in cnt if k.startswith(("sound", "SetEquipment")))[:5])
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:shears", dy=1.5)
    time.sleep(0.8)
    cnt = rec_stop(a, "shears", show=False)
    F("shears: click", ok, "body", sget("equipment"), "inv", {k: v for k, v in a.inv().items() if "harness" in k}, "fx", sorted(k for k in cnt if k.startswith(("sound", "SetEquipment", "ContainerSetSlot", "AddEntity")))[:6])
    # empty hand on a harnessed ghast = mount; with a lead
    for _ in range(2):
        sv(f"item replace entity {ME} weapon.mainhand with minecraft:air", show=False)
    use_on_entity(a, eid, "minecraft:stick", dy=1.5)
    F("after stick click: vehicle", a.state(0).get("vehicle"))


def lead():
    reset()
    sv("setblock 26 200 24 minecraft:oak_fence", show=False)
    sv("summon minecraft:happy_ghast 24.5 203 24.5 {PersistenceRequired:1b,equipment:{body:{id:\"minecraft:blue_harness\",count:1}}}", show=False)
    time.sleep(1.2)
    eid = ghasts()[0]["id"]
    give(ME, "minecraft:lead", 3)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:lead", dy=1.5)
    time.sleep(0.8)
    cnt = rec_stop(a, "lead", show=False)
    F("lead click", ok, "Leash", sget("leash"), "fx", sorted(k for k in cnt if k.startswith(("sound", "SetEntityLink", "EntityLink")))[:5])
    # tie to the fence with a real click on the fence
    a.select("minecraft:lead")
    ok2 = a.use_on(26, 200, 24)
    time.sleep(0.8)
    F("fence click", ok2, "Leash", sget("leash"))
    # the player walks away; the tethered ghast: max distance from the fence over 20 s
    sv(f"tp {ME} 26.5 200 30.5", show=False)
    mx, trk = 0.0, []
    for i in range(20):
        time.sleep(1.0)
        gp = gpos()
        d = round(math.hypot(gp[0] - 26.5, gp[2] - 24.5), 1) if gp else None
        trk.append(d)
    F("tethered ghast distance to the fence each second:", trk)
    F("ghast", gpos(), "leash entity", sget("leash"))


FN = {"follow": follow, "feed": feed, "stand": stand, "harness": harness, "lead": lead}
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "mob_griefing true", "do_daylight_cycle false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=14)
for ph in PH:
    try:
        FN[ph]()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR in", ph, repr(ex)[:300])
kill_all()
