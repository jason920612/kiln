"""wp68 #1: copper chests (open/close, double, waxing, scraping, comparator), wooden shelves (insert, redstone swap), copper tools and armour
(damage dealt / taken), lightning rods and lightning on copper; Kiln (TARGET=K) vs vanilla (TARGET=V), one actor, par.sh."""
from p68_small import *
import sys

scenario("copper2")


def items_of(path):
    out = sv(f"data get {path}", show=False, wait=0.5)
    return re.findall(r'(?:count: (\d+), Slot: (\d+)b, id: "minecraft:([a-z_]+)")|(?:id: "minecraft:([a-z_]+)", count: (\d+))', out)


def chest():
    reset()
    sv("fill 8 200 8 20 202 20 minecraft:air", show=False)
    sv(f"tp {ME} 14.5 200 14.5", show=False)
    time.sleep(1.0)
    give(ME, "minecraft:copper_chest", 3)
    give(ME, "minecraft:honeycomb", 2)
    give(ME, "minecraft:iron_axe", 1)
    give(ME, "minecraft:diamond", 5)
    a.select("minecraft:copper_chest")
    a.lookat(14.5, 199.99, 17.5)
    time.sleep(0.5)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("placed:", blk(a, 14, 200, 17))
    a.lookat(15.5, 199.99, 17.5)
    time.sleep(0.4)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("second placed next to it (double chest):", blk(a, 14, 200, 17), blk(a, 15, 200, 17), [l[:90] for l in lines() if "sound" in l][:4])
    # open the chest: a right click on it
    a.select("minecraft:diamond")
    a.lookat(14.5, 200.5, 17.5)
    time.sleep(0.5)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(1.2)
    F("open screen:", a.screen(), "slots:", len(a.state(0)["slots"]))
    st = a.state(0)
    dia = [s["i"] for s in st["slots"] if s["id"] == "minecraft:diamond"]
    if dia:
        a.guiclick(dia[0], "left", shift=True)
        time.sleep(0.4)
    a.key("esc", "tap")
    time.sleep(1.0)
    ls = lines()
    F("open/close sounds+events:", dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:90] for l in ls if "sound" in l or "block_event" in l)))
    F("chest items:", sorted((x[2] or x[3], x[0] or x[4]) for x in items_of("block 14 200 17 Items")), "comparator reading via execute:", sv("execute store result score #c p68c run data get block 14 200 17 Items[0].count", show=False)[-60:])
    # wax then scrape
    a.select("minecraft:honeycomb")
    a.lookat(14.5, 200.5, 17.5)
    time.sleep(0.4)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("waxed:", blk(a, 14, 200, 17), blk(a, 15, 200, 17), "screen:", a.screen(), dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:90] for l in lines() if "sound" in l or "level_event" in l)))
    if a.screen():
        a.key("esc", "tap")
    a.select("minecraft:iron_axe")
    time.sleep(0.4)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("scraped (wax off):", blk(a, 14, 200, 17), dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:90] for l in lines() if "sound" in l or "level_event" in l)))
    # oxidise by random ticks (only the unwaxed one moves)
    sv("gamerule random_tick_speed 4096", show=False)
    time.sleep(12)
    sv("gamerule random_tick_speed 3", show=False)
    F("after random ticks:", blk(a, 14, 200, 17), blk(a, 15, 200, 17))
    # break the double chest: contents drop
    sv("kill @e[type=item]", show=False)
    sv("setblock 14 200 17 minecraft:air destroy", show=False)
    time.sleep(1)
    F("items dropped from the broken chest:", dict(collections.Counter(e.get("item") for e in a.state(10)["entities"] if e["t"] == "item")))


def shelf():
    reset()
    sv("fill 8 200 8 24 202 24 minecraft:air", show=False)
    sv(f"tp {ME} 14.5 200 14.5", show=False)
    sv(f"clear {ME}", show=False)
    time.sleep(1.0)
    sv("setblock 14 200 18 minecraft:oak_shelf[facing=north]", show=False)
    sv("setblock 15 200 18 minecraft:oak_shelf[facing=north]", show=False)
    sv("setblock 16 200 18 minecraft:oak_shelf[facing=north]", show=False)
    time.sleep(0.5)
    for n, it in enumerate(("diamond", "iron_ingot", "gold_ingot", "emerald", "apple", "stick", "coal", "redstone", "lapis_lazuli")):
        sv(f"item replace entity {ME} hotbar.{n} with minecraft:{it} {n + 1}", show=False)
    F("shelf states:", blk(a, 14, 200, 18), blk(a, 15, 200, 18), blk(a, 16, 200, 18))
    # aim at the shelf front faces (z=18, south face at 18+1? the shelves face north: the player at z<18 sees their back; stand north)
    sv(f"tp {ME} 15.5 200 15.5", show=False)
    time.sleep(0.8)
    placed = []
    for n, x in enumerate((14.5, 15.5, 16.5)):
        a.hotbar(n + 1)
        a.lookat(x, 200.5, 18.0)
        time.sleep(0.4)
        rec_start(a)
        a.mouse("right", "tap")
        time.sleep(0.5)
        placed.append((n, a.state(0).get("hit"), [l[:70] for l in lines() if "sound" in l][:2]))
    F("hits/sounds per shelf click:", placed)
    F("shelf 1 items:", items_of("block 14 200 18 Items"))
    F("shelf 2 items:", items_of("block 15 200 18 Items"))
    F("shelf 3 items:", items_of("block 16 200 18 Items"))
    F("hotbar now:", sorted(k for k in a.inv().items() if k))
    # power the row: a redstone block beside it; then an empty-hand click swaps the hotbar with the shelf's contents
    sv("setblock 17 200 18 minecraft:redstone_block", show=False)
    time.sleep(0.6)
    F("powered:", blk(a, 14, 200, 18), blk(a, 15, 200, 18), blk(a, 16, 200, 18))
    a.hotbar(9)
    sv(f"item replace entity {ME} hotbar.8 with minecraft:air", show=False)
    a.lookat(15.5, 200.5, 18.0)
    time.sleep(0.4)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("swap click: sounds", dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:80] for l in lines() if "sound" in l or "SetSlot" in l or "ContainerSetContent" in l)))
    F("hotbar after the swap:", [re.sub(r".*(hotbar\.\d).*", r"\1", x) for x in []], sv(f"data get entity {ME} Inventory", show=False, wait=0.6)[-900:])
    for k in (14, 15, 16):
        F("shelf", k, items_of(f"block {k} 200 18 Items"))
    # empty-hand take of an item from an unpowered shelf
    sv("setblock 17 200 18 minecraft:air", show=False)
    time.sleep(0.5)
    a.hotbar(9)
    a.lookat(14.5, 200.5, 18.0)
    time.sleep(0.4)
    a.mouse("right", "tap")
    time.sleep(0.6)
    F("took from the unpowered shelf:", items_of("block 14 200 18 Items"), a.inv().get("minecraft:diamond"), a.inv().get("minecraft:emerald"))
    # breaking drops
    sv("kill @e[type=item]", show=False)
    sv("setblock 15 200 18 minecraft:air destroy", show=False)
    time.sleep(1)
    F("dropped:", dict(collections.Counter(e.get("item") for e in a.state(10)["entities"] if e["t"] == "item")))


def tools():
    reset()
    sv("fill 8 200 8 30 203 30 minecraft:air", show=False)
    sv(f"tp {ME} 14.5 200 14.5", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    time.sleep(1.0)
    rows = []
    for it in ("copper_sword", "copper_axe", "copper_pickaxe", "copper_shovel", "copper_hoe", "copper_spear", "iron_sword", "stone_sword", "golden_sword"):
        sv("kill @e[type=zombie]", show=False)
        sv("summon minecraft:zombie 14.5 200 17.0 {NoAI:1b,Silent:1b,Health:100f,Attributes:[{id:'minecraft:max_health',base:100d}],PersistenceRequired:1b}", show=False)
        time.sleep(0.8)
        sv(f"item replace entity {ME} weapon.mainhand with minecraft:{it}", show=False)
        a.lookat(14.5, 201.0, 17.0)
        time.sleep(1.4)
        a.mouse("left", "tap")
        time.sleep(0.5)
        h = re.findall(r"Health: ([\d.]+)f", sv("data get entity @e[type=zombie,limit=1]", show=False, wait=0.4))
        dmg = sv(f"data get entity {ME} SelectedItem", show=False, wait=0.4)
        rows.append((it, h, re.findall(r"damage: (\d+)", dmg)))
    F("one full-strength hit (zombie health left, item damage):", rows)
    # armour: the same hit taken
    sv("kill @e[type=zombie]", show=False)
    rows = []
    for mat in ("copper", "iron", "golden", "leather", "chainmail"):
        sv(f"clear {ME}", show=False)
        for slot, p in (("armor.head", "helmet"), ("armor.chest", "chestplate"), ("armor.legs", "leggings"), ("armor.feet", "boots")):
            sv(f"item replace entity {ME} {slot} with minecraft:{mat}_{p}", show=False)
        sv(f"effect clear {ME}", show=False)
        sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
        time.sleep(0.6)
        sv(f"damage {ME} 10 minecraft:generic", show=False)
        time.sleep(0.4)
        hp = re.findall(r"Health: ([\d.]+)f", sv(f"data get entity {ME}", show=False, wait=0.4))
        arm = re.findall(r"id: \"minecraft:armor\"[^}]*", sv(f"data get entity {ME} attributes", show=False, wait=0.4))
        rows.append((mat, hp, [re.sub(r"[^\d.]", "", x)[:6] for x in arm][:1]))
        sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
        time.sleep(0.4)
    F("10 damage through four pieces (health left):", rows)


def rod():
    reset()
    sv("fill 8 200 8 30 205 30 minecraft:air", show=False)
    sv(f"tp {ME} 10.5 200 10.5", show=False)
    time.sleep(1.0)
    # a rod with a lamp and a comparator-free redstone line, struck; copper blocks under a bolt oxidise/wax state
    sv("setblock 20 200 20 minecraft:lightning_rod[facing=up]", show=False)
    sv("setblock 20 200 22 minecraft:redstone_lamp", show=False)
    sv("setblock 20 200 21 minecraft:redstone_wire", show=False)
    sv("setblock 22 200 20 minecraft:oxidized_copper", show=False)
    sv("setblock 22 201 20 minecraft:waxed_oxidized_copper", show=False)
    sv("setblock 24 200 20 minecraft:weathered_cut_copper", show=False)
    time.sleep(0.5)
    sv("gamerule mob_griefing true", show=False)
    rec_start(a)
    sv("summon minecraft:lightning_bolt 21.5 200 24.5", show=False)  # a bolt 4 blocks off: the rod draws it? (only in the natural strike)
    time.sleep(1.0)
    F("bolt next to the rod:", blk(a, 20, 200, 20))
    sv("summon minecraft:lightning_bolt 20.5 201 20.5", show=False)
    seen = []
    for _ in range(14):
        seen.append(blk(a, 20, 200, 20)[-24:])
        time.sleep(0.15)
    F("rod powered over time:", seen)
    F("lamp:", blk(a, 20, 200, 22))
    sv("summon minecraft:lightning_bolt 22.5 201 20.5", show=False)
    time.sleep(1.0)
    F("copper under a bolt (oxidized, waxed above):", blk(a, 22, 200, 20), blk(a, 22, 201, 20))
    sv("setblock 24 200 20 minecraft:exposed_copper", show=False)
    sv("summon minecraft:lightning_bolt 24.5 201 20.5", show=False)
    time.sleep(1.0)
    F("exposed copper under a bolt:", blk(a, 24, 200, 20))
    ls = lines()
    F("bolt sounds/events:", dict(collections.Counter(re.sub(r"vol=\S+|pitch=\S+", "", l)[:80] for l in ls if "sound" in l or "level_event" in l or "AddEntity" in l)))
    # natural strike: thunderstorm, the rod nearest within 128 blocks takes it (summon-free: `weather thunder`, tick sprint a little)
    sv("weather thunder 100000", show=False)
    sv("gamerule random_tick_speed 3", show=False)
    # near a player a skeleton-horse trap or a bolt hits a random spot; the rod attracts the strike in a 128 block radius
    sv("tick sprint 3000", show=False, wait=0.4)
    for _ in range(80):
        time.sleep(1)
        if "sprinting" not in sv("tick query", show=False, wait=0.3).lower():
            break
    F("rod after a thunderstorm sprint:", blk(a, 20, 200, 20))
    sv("weather clear", show=False)


if __name__ == "__main__":
    prep()
    time.sleep(2)
    sv("scoreboard objectives add p68c dummy", show=False)
    for which in sys.argv[1:] or ["chest", "shelf", "tools", "rod"]:
        try:
            globals()[which]()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
