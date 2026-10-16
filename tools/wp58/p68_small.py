"""wp68 #1: golden dandelion (age lock on babies, brown mooshroom stew), mannequin (summon, hit, name tag), Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p68 import *
import sys

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
scenario("small")
ME = "KilnTest"

def prep():
    if T == "V":
        sv("op KilnTest", show=False)
    alive(a)
    sv("gamerule spawn_mobs false", show=False)
    sv("gamerule mob_griefing true", show=False)
    sv("difficulty normal", show=False)
    sv("time set day", show=False)
    sv("weather clear", show=False)
    platform(0, 0, 40, 40, y=199, clear=12)
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(2.5)

def summon(typ, nbt="{}", x=20.5, z=23.5, y=200):
    before = {e["id"] for e in a.state(40)["entities"]}
    sv(f"summon minecraft:{typ} {x} {y} {z} {nbt}", show=False)
    time.sleep(1.2)
    new = [e for e in a.state(40)["entities"] if e["id"] not in before]
    return new[0] if new else None

def reset():
    clear_area()
    if a.screen() == "DeathScreen" or a.state(0)["health"] <= 0:
        alive(a)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    a.release()
    time.sleep(0.6)

def lines():
    return json.loads(a.cmd("spylog clear"))

def flt(ls, *keys):
    return [l[:140] for l in ls if any(k in l for k in keys)]

def dandelion():
    for typ, nbt in (("cow", "{Age:-24000}"), ("pig", "{Age:-24000}"), ("horse", "{Age:-24000}"), ("villager", "{Age:-24000}"), ("zombie", "{IsBaby:1b,NoAI:1b}"), ("sheep", "{}"), ("frog", "{}")):
        reset()
        m = summon(typ, nbt)
        if not m:
            F("dandelion", typ, "not summoned"); continue
        eid = m["id"]
        give(ME, "minecraft:golden_dandelion", 4)
        rec_start(a)
        ok = use_on_entity(a, eid, "minecraft:golden_dandelion")
        time.sleep(1.0)
        ls = lines()
        F("dandelion", typ, "ok", ok, "left", a.inv().get("minecraft:golden_dandelion", 0), "entdata", a.cmd(f"entdata {eid}")[:90], flt(ls, "sound", "particle")[:5])
        # second use unlocks
        if ok:
            time.sleep(3.2)  # the age lock particle cooldown
            ok2 = use_on_entity(a, eid, "minecraft:golden_dandelion")
            time.sleep(0.8)
            F("dandelion again", typ, "ok", ok2, "left", a.inv().get("minecraft:golden_dandelion", 0), "entdata", a.cmd(f"entdata {eid}")[:90], flt(lines(), "sound")[:3])
    # locked baby: wheat does not age it, and it does not grow up with time
    reset()
    m = summon("cow", "{Age:-100,AgeLocked:1b}")
    give(ME, "minecraft:wheat", 4)
    ok = use_on_entity(a, m["id"], "minecraft:wheat")
    F("locked cow fed wheat ok", ok, "wheat left", a.inv().get("minecraft:wheat", 0))
    time.sleep(8)
    F("locked cow after 8 s still baby:", [(e.get("baby")) for e in ents_of(a, "cow")])
    sv(f"data get entity @e[type=cow,limit=1] Age", show=False)
    reset()
    # brown mooshroom: flower + bowl
    m = summon("mooshroom", '{Type:"brown"}')
    give(ME, "minecraft:golden_dandelion", 1); give(ME, "minecraft:bowl", 1)
    ok = use_on_entity(a, m["id"], "minecraft:golden_dandelion")
    ok2 = use_on_entity(a, m["id"], "minecraft:bowl")
    F("brown mooshroom flower", ok, "bowl", ok2, {k: v for k, v in a.inv().items() if "stew" in k or "bowl" in k or "dandelion" in k})
    st = sv(f"data get entity {ME} Inventory", show=False)
    F("stew item:", re.findall(r"suspicious_stew[^}]*\}[^}]*\}", st)[:1])

def mannequin():
    reset()
    m = summon("mannequin", '{profile:{name:"Steve"},description:"Hello",pose:"standing"}')
    if not m:
        F("mannequin not summoned"); return
    eid = m["id"]
    F("mannequin client:", {k: m[k] for k in ("pose", "bbh", "bbw", "hp") if k in m}, "entdata", a.cmd(f"entdata {eid}")[:300])
    F("ent:", a.cmd(f"ent {eid}")[:400])
    # hit with fists, then a sword
    hp0 = m["hp"]
    rec_start(a)
    interact_ent(a, eid, button="left", tries=3, dy=0.9)
    time.sleep(1.0)
    e1 = [e for e in a.state(40)["entities"] if e["id"] == eid]
    F("mannequin punched: hp", hp0, "->", e1[0]["hp"] if e1 else None, "moved z", (round(e1[0]["z"], 2) if e1 else None), flt(lines(), "sound", "damage", "Damage", "hurt", "entity_event")[:5])
    # name tag, armor right-click
    give(ME, "minecraft:name_tag", 1)
    sv(f"data merge entity {ME} {{}}", show=False)
    give(ME, "minecraft:iron_helmet", 1)
    ok = use_on_entity(a, eid, "minecraft:iron_helmet", dy=0.9)
    F("helmet right click ok", ok, "inv helmet", a.inv().get("minecraft:iron_helmet", 0), "ent", a.cmd(f"ent {eid}")[:200])
    # kill it with a diamond sword: loot
    give(ME, "minecraft:diamond_sword", 1)
    a.select("minecraft:diamond_sword")
    for _ in range(6):
        interact_ent(a, eid, button="left", tries=2, dy=0.9)
        time.sleep(0.7)
        if not [e for e in a.state(40)["entities"] if e["id"] == eid]:
            break
    F("mannequin after sword: alive?", bool([e for e in a.state(40)["entities"] if e["id"] == eid]), "drops:", [(e.get("item"), e.get("n")) for e in a.state(10)["entities"] if e["t"] == "item"])
    reset()
    # immovable + poses
    for pose in ("standing", "crouching", "swimming", "fall_flying", "sleeping"):
        m = summon("mannequin", '{pose:"%s",immovable:1b}' % pose)
        F("mannequin pose", pose, m and {k: m[k] for k in ("pose", "bbh", "bbw")})
        clear_area()

if __name__ == "__main__":
    prep()
    time.sleep(4)
    for fn in sys.argv[1:] or ["dandelion", "mannequin"]:
        try:
            globals()[fn]()
        except Exception as ex:
            import traceback; traceback.print_exc()
            F(fn, "ERROR", repr(ex)[:200])
