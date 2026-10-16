"""wp68 #3: nautilus and zombie nautilus (taming, saddle, armour, riding underwater, dash), camel husk (saddle, ride, dash, sit) and the parched
skeleton, Kiln (TARGET=K) vs vanilla (TARGET=V). One actor; run with par.sh."""
from p68_small import *
import sys

scenario("mounts")


def pool():
    """A big water pool cut into the platform: stone box (9..39, 188..198), water y 189..198 inside (10..38)."""
    sv("fill 9 188 9 39 198 39 minecraft:stone", show=False)
    sv("fill 10 189 10 38 198 38 minecraft:water", show=False)
    sv("fill 9 199 9 39 205 39 minecraft:air", show=False)
    time.sleep(0.5)


def eid_of(typ):
    es = ents_of(a, typ, 40)
    return es[0]["id"] if es else None


def has_data(typ, path):
    out = sv(f"execute if data entity @e[type={typ},limit=1] {path}", show=False)
    return "pass" in out and "fail" not in out


def srv(typ, path):
    return sv(f"data get entity @e[type={typ},limit=1] {path}", show=False)


def nautilus_tame(typ="nautilus"):
    reset()
    pool()
    sv(f"tp {ME} 24.5 200 8.5", show=False)
    time.sleep(2.0)
    sv(f"summon minecraft:{typ} 24.5 195 15.5 {{PersistenceRequired:1b}}", show=False)
    time.sleep(1.5)
    eid = eid_of(typ)
    F(typ, "summoned:", eid, a.cmd(f"entdata {eid}")[:100] if eid else None, "bb", [(e["bbw"], e["bbh"]) for e in ents_of(a, typ, 40)])
    give(ME, "minecraft:pufferfish", 16)
    give(ME, "minecraft:saddle", 1)
    n = 0
    for _ in range(14 if "realtame" in sys.argv else 0):
        # freeze it for the click (the nautilus swims about)
        sv(f"data merge entity @e[type={typ},limit=1] {{NoAI:1b}}", show=False)
        ok = use_on_entity(a, eid, "minecraft:pufferfish", dy=0.3)
        n += 1
        if has_data(typ, "Owner"):
            break
        sv(f"data merge entity @e[type={typ},limit=1] {{NoAI:0b}}", show=False)
        time.sleep(0.5)
    if not has_data(typ, "Owner"):
        sv(f"data modify entity @e[type={typ},limit=1] Owner set from entity {ME} UUID", show=False)
    F(typ, "tamed after", n, "clicks, owner:", re.findall(r"Owner: \[I; [-0-9, ]+\]", srv(typ, "")), "pufferfish left server:", re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:pufferfish"', sv(f"data get entity {ME} Inventory", show=False)))
    return eid


def nautilus_gear(typ="nautilus"):
    eid = eid_of(typ)
    sv(f"data merge entity @e[type={typ},limit=1] {{NoAI:1b}}", show=False)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:saddle", dy=0.3)
    time.sleep(0.8)
    F(typ, "saddle ok", ok, "server equipment:", re.findall(r"saddle: \{[^}]*\}", srv(typ, "")), [l[:90] for l in lines() if "equipment" in l or "sound" in l][:4])
    for armor in ("minecraft:iron_nautilus_armor", "minecraft:copper_nautilus_armor", "minecraft:diamond_nautilus_armor"):
        give(ME, armor, 1)
        ok = use_on_entity(a, eid, armor, dy=0.3)
        time.sleep(0.6)
        F(typ, armor.split(":")[1], "ok", ok, "server body:", re.findall(r"body: \{[^}]*\}", srv(typ, "")), "left:", a.inv().get(armor, 0))
        sv(f"item replace entity @e[type={typ},limit=1] armor.body with minecraft:air", show=False)
    # armour back for the ride
    give(ME, "minecraft:golden_nautilus_armor", 1)
    use_on_entity(a, eid, "minecraft:golden_nautilus_armor", dy=0.3)
    time.sleep(0.5)
    F(typ, "attributes (armour):", re.findall(r'\{id: "minecraft:armor"[^}]*\}', srv(typ, "attributes")))


def nautilus_ride(typ="nautilus"):
    eid = eid_of(typ)
    sv(f"item replace entity @e[type={typ},limit=1] saddle with minecraft:saddle", show=False)
    sv(f"data merge entity @e[type={typ},limit=1] {{NoAI:0b}}", show=False)
    time.sleep(0.5)
    sv(f"tp @e[type={typ}] 24.5 195 15.5", show=False)
    sv(f"tp {ME} 24.5 200 12.5", show=False)
    time.sleep(1.5)
    ok = interact_ent(a, eid, button="right", tries=10, dy=0.3)
    time.sleep(1.0)
    F(typ, "ride ok", ok, "vehicle", a.state(0)["vehicle"], pos(a))
    if not a.state(0)["vehicle"]:
        sv(f"ride {ME} mount @e[type={typ},limit=1]", show=False)
        time.sleep(1.0)
        F(typ, "ride by command:", a.state(0)["vehicle"])
    if not a.state(0)["vehicle"]:
        return
    g0 = [e for e in ents_of(a, typ, 40) if e["id"] == eid][0]
    p0 = (g0["x"], g0["y"], g0["z"])
    a.look(0, 0)
    a.key("w", "down")
    time.sleep(2.0)
    a.key("w", "up")
    g1 = [e for e in ents_of(a, typ, 40) if e["id"] == eid][0]
    F(typ, "swam 2 s forward:", [round(g1[k] - p0[i], 2) for i, k in enumerate(("x", "y", "z"))], "server", re.findall(r"Pos: \[[^\]]*\]", srv(typ, "")))
    # dash: hold jump, release
    p0 = (g1["x"], g1["y"], g1["z"])
    a.look(180, 0)
    a.key("space", "down")
    time.sleep(1.2)
    a.key("space", "up")
    time.sleep(1.5)
    g2 = [e for e in ents_of(a, typ, 40) if e["id"] == eid][0]
    F(typ, "dash (jump held 1.2 s then released):", [round(g2[k] - p0[i], 2) for i, k in enumerate(("x", "y", "z"))], [l[:90] for l in lines() if "sound" in l and "swim" not in l][:5], "entdata", a.cmd(f"entdata {eid}")[:100])
    # leave
    a.key("shift", "down")
    time.sleep(0.3)
    a.key("shift", "up")
    time.sleep(1.0)
    F(typ, "dismount:", a.state(0)["vehicle"], pos(a))


def camel_husk():
    reset()
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 22.5 200 22.5", show=False)
    time.sleep(1.5)
    sv("summon minecraft:camel_husk 25.5 200 25.5 {PersistenceRequired:1b,NoAI:1b}", show=False)
    time.sleep(1.5)
    eid = eid_of("camel_husk")
    F("camel_husk:", eid, [(e["bbw"], e["bbh"]) for e in ents_of(a, "camel_husk", 40)], a.cmd(f"entdata {eid}")[:100] if eid else None)
    give(ME, "minecraft:saddle", 1)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:saddle", dy=1.0)
    time.sleep(0.8)
    F("camel_husk saddle ok", ok, "server:", re.findall(r"saddle: \{[^}]*\}", srv("camel_husk", "")), [l[:90] for l in lines() if "equipment" in l or "sound" in l][:3])
    ok = interact_ent(a, eid, button="right", tries=10, dy=1.0)
    time.sleep(1.0)
    F("camel_husk ride", ok, a.state(0)["vehicle"], pos(a), "ent:", json.loads(a.cmd(f"ent {eid}")).get("passengers"))
    if a.state(0)["vehicle"]:
        sv("data merge entity @e[type=camel_husk,limit=1] {NoAI:0b}", show=False)
        time.sleep(0.5)
        g0 = ents_of(a, "camel_husk", 40)[0]
        p0 = (g0["x"], g0["z"])
        a.look(0, 0)
        a.key("w", "down")
        time.sleep(2.0)
        a.key("w", "up")
        g1 = ents_of(a, "camel_husk", 40)[0]
        F("camel_husk walked 2 s:", round(math.hypot(g1["x"] - p0[0], g1["z"] - p0[1]), 2))
        a.key("ctrl", "down")
        a.key("w", "down")
        time.sleep(2.0)
        a.key("w", "up")
        a.key("ctrl", "up")
        g2 = ents_of(a, "camel_husk", 40)[0]
        F("camel_husk sprint 2 s:", round(math.hypot(g2["x"] - g1["x"], g2["z"] - g1["z"]), 2))
        # dash: hold jump then release
        a.key("w", "down")
        a.key("space", "down")
        time.sleep(1.2)
        a.key("space", "up")
        time.sleep(0.4)
        a.key("w", "up")
        time.sleep(1.5)
        g3 = ents_of(a, "camel_husk", 40)[0]
        F("camel_husk dash:", round(math.hypot(g3["x"] - g2["x"], g3["z"] - g2["z"]), 2), a.cmd(f"entdata {eid}")[:100])
        a.key("shift", "down")
        time.sleep(0.3)
        a.key("shift", "up")
        time.sleep(1.0)
        F("camel_husk dismount", a.state(0)["vehicle"], pos(a))


def parched():
    reset()
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    sv(f"effect clear {ME}", show=False)
    time.sleep(1.5)
    sv("summon minecraft:parched 20.5 200 28.5 {PersistenceRequired:1b}", show=False)
    time.sleep(1.5)
    es = ents_of(a, "parched", 40)
    F("parched:", [(e["id"], e["bbw"], e["bbh"], e["hp"]) for e in es], a.cmd(f"entdata {es[0]['id']}")[:100] if es else None)
    if es:
        F("parched equipment:", a.cmd(f"ent {es[0]['id']}")[:260])
    rec_start(a)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    t0 = time.time()
    arrows = 0
    hit_effects = None
    while time.time() - t0 < 20:
        time.sleep(0.5)
        arrows = max(arrows, len(ents_of(a, "arrow", 40)) + len(ents_of(a, "tipped_arrow", 40)) + len(ents_of(a, "spectral_arrow", 40)))
        fx = a.state(0)["effects"]
        if any("weakness" in f["id"] for f in fx):
            hit_effects = fx
            break
    F("parched shot:", "max arrows seen", arrows, "effects after a hit:", hit_effects, "health", a.state(0)["health"])
    types = collections.Counter(e["t"] for e in a.state(60)["entities"])
    F("entity types around:", dict(types))


if __name__ == "__main__":
    prep()
    time.sleep(3)
    for which in sys.argv[1:] or ["nautilus", "zombie_nautilus", "camel_husk", "parched"]:
        try:
            if which in ("nautilus", "zombie_nautilus"):
                nautilus_tame(which)
                nautilus_gear(which)
                nautilus_ride(which)
            else:
                globals()[which]()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
