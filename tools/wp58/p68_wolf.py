"""wp68 #5: wolf armour: equip by right click, the armour takes damage instead of the wolf, repair with armadillo scutes, dye in a crafting grid; Kiln vs vanilla."""
from p68_small import *
import sys

scenario("wolf")


def wolf_state():
    out = sv("data get entity @e[type=wolf,limit=1]", show=False)
    return {"hp": re.findall(r"Health: ([\d.]+)f", out), "body": re.findall(r'body: \{[^}]*\}', out)[:1], "dmg": re.findall(r"damage: (\d+)", out)[:1], "attr": re.findall(r'\{id: "minecraft:armor"[^}]*\}', out)[:1]}


def run():
    prep()
    time.sleep(2)
    platform(0, 0, 40, 40, y=199, clear=12)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    sv("clear KilnTest", show=False)
    time.sleep(1.0)
    sv("summon minecraft:wolf 20.5 200 23.5 {NoAI:1b,PersistenceRequired:1b}", show=False)
    sv("data modify entity @e[type=wolf,limit=1] Owner set from entity KilnTest UUID", show=False)
    time.sleep(1.2)
    eid = ents_of(a, "wolf", 20)[0]["id"]
    F("wolf:", a.cmd(f"entdata {eid}")[:100], wolf_state())
    give(ME, "minecraft:wolf_armor", 1)
    give(ME, "minecraft:armadillo_scute", 8)
    give(ME, "minecraft:diamond_sword", 1)
    give(ME, "minecraft:red_dye", 1)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:wolf_armor", dy=0.5)
    time.sleep(0.8)
    F("wolf armor equip ok", ok, wolf_state(), "left", a.inv().get("minecraft:wolf_armor", 0), [l[:90] for l in lines() if "sound" in l or "equipment" in l][:4])
    # hit it
    a.select("minecraft:diamond_sword")
    for i in range(4):
        interact_ent(a, eid, button="left", tries=3, dy=0.5)
        time.sleep(1.0)
        F("after hit", i + 1, wolf_state())
    # repair with scutes
    a.select("minecraft:armadillo_scute")
    rec_start(a)
    for i in range(3):
        ok = interact_ent(a, eid, button="right", tries=4, dy=0.5)
        time.sleep(0.7)
        F("scute", i + 1, "ok", ok, wolf_state(), "scutes left", a.inv().get("minecraft:armadillo_scute", 0))
    F("repair sounds:", [l[:80] for l in lines() if "sound" in l][:5])


if __name__ == "__main__":
    try:
        run()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
