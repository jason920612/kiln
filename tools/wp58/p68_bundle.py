"""wp68 #5: bundles in the real inventory screen: insert by clicking the item onto the bundle, take out with a right click, select an item
with the mouse wheel (bundle_item_selected), drop everything with the use key; the contents the server holds. Kiln vs vanilla."""
from p68_small import *
import sys

scenario("bundle")


def server_bundle(slot_path="Inventory"):
    out = sv(f"data get entity {ME} Inventory", show=False, wait=0.8)
    return re.findall(r'(?:minecraft:bundle_contents|bundle_contents)": (\[.*?\])\}?[,}]', out)[:1] or re.findall(r"bundle_contents.{0,300}", out)[:1]


def inv_screen():
    a.key("e", "tap")
    time.sleep(1.0)
    return a.screen()


def slot_of(menu_slots, item):
    for s in menu_slots:
        if s["id"] == item:
            return s["i"]
    return None


def run():
    prep()
    time.sleep(2)
    if a.screen():
        a.key("esc", "tap")
        time.sleep(0.6)
    sv(f"clear {ME}", show=False)
    give(ME, "minecraft:bundle", 1)
    give(ME, "minecraft:cobblestone", 64)
    give(ME, "minecraft:stick", 10)
    give(ME, "minecraft:diamond", 3)
    give(ME, "minecraft:bundle", 1)
    time.sleep(0.8)
    F("screen:", inv_screen())
    st = a.state(0)
    slots = st["slots"]
    # the player inventory menu: 0 craft out, 1-4 craft, 5-8 armour, 9-35 inventory, 36-44 hotbar, 45 offhand
    F("slots with items:", [(s["i"], s["id"].split(":")[1], s["n"]) for s in slots if s["id"]][:12])
    b1 = slot_of(slots, "minecraft:bundle")
    cob = slot_of(slots, "minecraft:cobblestone")
    stick = slot_of(slots, "minecraft:stick")
    dia = slot_of(slots, "minecraft:diamond")
    F("bundle slot", b1, "cobble", cob, "stick", stick, "diamond", dia)
    rec_start(a)
    # the second bundle (slot 40) takes sticks and diamonds, and some cobblestone: three kinds
    b2 = 40
    rec_start(a)
    for src, n in ((stick, "stick"), (dia, "diamond"), (cob, "cobble")):
        a.guiclick(src, "left")
        a.guiclick(b2, "left")
        # whatever is left on the cursor goes back into its slot
        if a.state(0).get("carried", "").split(" x")[0] not in ("minecraft:air", ""):
            a.guiclick(src, "left")
        time.sleep(0.2)
    time.sleep(0.6)
    F("client bundle 2 after inserting:", a.cmd(f"bundlesel {b2}"))
    F("carried after the clicks:", a.state(0).get("carried"))
    F("insert sounds/packets:", [l[:80] for l in lines() if "sound" in l or "SetSlot" in l or "Container" in l][:8])
    F("server inventory (bundle contents):", server_bundle())
    b1 = b2
    # wheel selection: hover the bundle, scroll
    a.cmd(f"guihover {b1}")
    time.sleep(0.5)
    for d in (-1, -1, 1, 1, 1):
        rec_start(a)
        a.cmd(f"scroll {d}")
        time.sleep(0.6)
        F("scroll", d, "-> client selected:", a.cmd(f"bundlesel {b1}"), "packets:", [l[:70] for l in lines() if "Bundle" in l or "sound" in l][:3])
    # right click the bundle in the screen: takes the selected item out onto the cursor
    rec_start(a)
    a.guiclick(b1, "right")
    time.sleep(0.6)
    F("right click on the bundle: carried", a.state(0).get("carried"), "bundle now", a.cmd(f"bundlesel {b1}"), [l[:70] for l in lines() if "sound" in l][:3])
    # put it back, close, use the bundle in hand: drops all
    a.guiclick(b1, "left")
    a.key("esc", "tap")
    time.sleep(0.8)
    F("screen after esc:", a.screen())
    st = a.state(0)
    hot = [e for e in st["inv"] if e["id"] == "minecraft:bundle"]
    F("bundles in the inventory:", hot)
    if hot:
        slot = hot[0]["slot"]
        if slot < 9:
            a.hotbar(slot + 1)
        else:
            a.select("minecraft:bundle")
        rec_start(a)
        a.mouse("right", "down")
        time.sleep(1.5)
        a.mouse("right", "up")
        time.sleep(1.0)
        F("dropped by holding the use key:", dict(collections.Counter((e["item"]) for e in a.state(8)["entities"] if e["t"] == "item")), [l[:70] for l in lines() if "sound" in l][:4])
        F("inventory after:", {k: v for k, v in a.inv().items()})


def slots_index(x):
    return x


if __name__ == "__main__":
    try:
        run()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
