"""wp84 #4: decorated pots with a real client, one actor against each server (par.sh): putting items in (the same item adds, another one shakes the pot's head, an empty
hand shakes), the wobble block events and sounds, the comparator beside it, a hopper emptying it, an arrow breaking it, a pickaxe breaking it (what drops: the sherds and the
stored items), the sherds crafted into a pot at a real crafting table.
usage: par.sh p84_pots.py [phases: insert,hopper,break,craft]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
PH = (sys.argv[1] if len(sys.argv) > 1 else "insert,hopper,break,craft").split(",")
scenario("pots84")
POT = (20, 200, 24)


def norm(cnt):
    return sorted(k for k in cnt if k.startswith(("sound", "block_event", "ContainerSetSlot", "particle", "level_event", "BlockEvent", "block_update")))[:14]


def pot_data():
    return re.sub(r"\s+", " ", sv(f"data get block {POT[0]} {POT[1]} {POT[2]}", show=False, wait=0.6))[-250:]


def reset(extra=""):
    kill_all()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"fill 15 200 18 26 204 30 minecraft:air", show=False)
    sv(f"setblock {POT[0]} {POT[1]} {POT[2]} minecraft:decorated_pot[facing=north]{extra}", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.2)


def click_with(item, label):
    if item:
        a.select(item)
    else:
        sv(f"item replace entity {ME} weapon.mainhand with minecraft:air", show=False)
    a.lookat(POT[0] + 0.5, POT[1] + 0.5, POT[2] + 0.5)
    time.sleep(0.4)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(0.8)
    cnt = rec_stop(a, label, show=False)
    F(label, "hit", a.state(0).get("hit", {}).get("id") if a.state(0).get("hit") else None, "pot", pot_data()[-170:], "inv", {k.split(":")[1]: v for k, v in a.inv().items() if k.split(":")[1] in ("stone", "dirt", "arrow")}, "fx", norm(cnt))


def insert():
    reset()
    sv(f"give {ME} minecraft:stone 64", show=False)
    sv(f"give {ME} minecraft:dirt 64", show=False)
    sv(f"give {ME} minecraft:arrow 4", show=False)
    time.sleep(0.5)
    click_with("minecraft:stone", "i1_stone")
    click_with("minecraft:stone", "i2_stone")
    click_with("minecraft:dirt", "i3_dirt_into_stone")
    click_with(None, "i4_empty_hand")
    # a comparator behind the pot: the signal is the stack's fullness
    sv(f"setblock {POT[0]} {POT[1]} {POT[2] - 1} minecraft:comparator[facing=south]", show=False)
    sv(f"setblock {POT[0] + 1} {POT[1]} {POT[2] - 1} minecraft:redstone_wire", show=False)
    time.sleep(0.6)
    F("comparator after 2 stones:", sv(f"data get block {POT[0]} {POT[1]} {POT[2] - 1} OutputSignal", show=False)[-60:], blk(a, POT[0], POT[1], POT[2] - 1)[:60], blk(a, POT[0] + 1, POT[1], POT[2] - 1)[:40])
    for _ in range(40):
        click_with("minecraft:stone", "i5_more_stone") if _ in (0, 20) else None
        sv(f"item replace entity {ME} weapon.mainhand with minecraft:stone 64", show=False)
        a.select("minecraft:stone")
        a.lookat(POT[0] + 0.5, POT[1] + 0.5, POT[2] + 0.5)
        a.mouse("right", "tap")
        time.sleep(0.12)
        if "count: 64" in pot_data() or "count: 63" in pot_data() or "count: 62" in pot_data():
            break
    F("pot filled with stone:", pot_data()[-120:], "comparator", sv(f"data get block {POT[0]} {POT[1]} {POT[2] - 1} OutputSignal", show=False)[-40:])


def hopper():
    reset()
    sv(f"data merge block {POT[0]} {POT[1]} {POT[2]} {{item:{{id:\"minecraft:stone\",count:30}}}}", show=False)
    sv(f"setblock {POT[0]} {POT[1] - 1} {POT[2]} minecraft:hopper[facing=down]", show=False)
    sv(f"setblock {POT[0]} {POT[1] - 2} {POT[2]} minecraft:chest", show=False)
    time.sleep(0.5)
    F("hopper under the pot: pot", pot_data()[-120:])
    time.sleep(6.0)
    F("after 6 s: pot", pot_data()[-120:], "chest", re.sub(r"\s+", " ", sv(f"data get block {POT[0]} {POT[1] - 2} {POT[2]} Items", show=False))[-160:])
    # a hopper feeding the pot from the side (an item dropped into the hopper)
    sv(f"setblock {POT[0]} {POT[1] - 1} {POT[2]} minecraft:air", show=False)
    sv(f"setblock {POT[0]} {POT[1] - 2} {POT[2]} minecraft:air", show=False)
    sv(f"setblock {POT[0]} {POT[1]} {POT[2] + 1} minecraft:hopper[facing=north]", show=False)
    sv(f"summon minecraft:item {POT[0]}.5 {POT[1] + 1}.5 {POT[2] + 1}.5 {{Item:{{id:\"minecraft:stone\",count:10}}}}", show=False)
    time.sleep(5.0)
    F("a hopper feeds the pot: pot", pot_data()[-120:])


def breaking():
    reset("")
    sv(f"data merge block {POT[0]} {POT[1]} {POT[2]} {{item:{{id:\"minecraft:dirt\",count:5}},sherds:[\"minecraft:arms_up_pottery_sherd\",\"minecraft:brick\",\"minecraft:skull_pottery_sherd\",\"minecraft:brick\"]}}", show=False)
    time.sleep(0.4)
    F("pot with sherds:", pot_data()[-200:])
    sv(f"give {ME} minecraft:iron_pickaxe 1", show=False)
    a.select("minecraft:iron_pickaxe")
    rec_start(a)
    a.mine(POT[0], POT[1], POT[2], timeout=15)
    time.sleep(1.0)
    cnt = rec_stop(a, "break_pickaxe", show=False)
    items = collections.Counter()
    for e in a.state(10)["entities"]:
        if e["t"] == "item":
            items[e.get("item")] += e.get("n", 1)
    F("broken with an iron pickaxe: block", blk(a, *POT), "items on the floor", dict(items), "fx", norm(cnt))
    # silk touch
    reset()
    sv(f"data merge block {POT[0]} {POT[1]} {POT[2]} {{item:{{id:\"minecraft:dirt\",count:5}},sherds:[\"minecraft:arms_up_pottery_sherd\",\"minecraft:brick\",\"minecraft:skull_pottery_sherd\",\"minecraft:brick\"]}}", show=False)
    sv(f"give {ME} minecraft:iron_pickaxe[enchantments={{\"minecraft:silk_touch\":1}}] 1", show=False)
    a.select("minecraft:iron_pickaxe")
    a.mine(POT[0], POT[1], POT[2], timeout=15)
    time.sleep(1.0)
    items = collections.Counter()
    for e in a.state(10)["entities"]:
        if e["t"] == "item":
            items[e.get("item")] += e.get("n", 1)
    F("broken with silk touch: items on the floor", dict(items))
    # an arrow breaks it (cracks, then breaks)
    reset()
    sv(f"give {ME} minecraft:bow 1", show=False)
    sv(f"give {ME} minecraft:arrow 8", show=False)
    a.select("minecraft:bow")
    sv(f"tp {ME} 20.5 200 30.5", show=False)
    time.sleep(1.0)
    rec_start(a)
    a.lookat(POT[0] + 0.5, POT[1] + 0.5, POT[2] + 0.5)
    a.mouse("right", "down"); time.sleep(1.3); a.mouse("right", "up")
    time.sleep(1.5)
    cnt = rec_stop(a, "break_arrow", show=False)
    F("arrow at the pot: block", blk(a, *POT), "fx", norm(cnt))


def craft():
    reset()
    sv("setblock 20 200 22 minecraft:crafting_table", show=False)
    for it in ("arms_up_pottery_sherd", "skull_pottery_sherd", "danger_pottery_sherd", "brick"):
        sv(f"give {ME} minecraft:{it} 1", show=False)
    time.sleep(0.5)
    try:
        # diamond: top, left, right, bottom
        a.craft_table((20, 200, 22), [" A ", "S D", " B "], {"A": "minecraft:arms_up_pottery_sherd", "S": "minecraft:skull_pottery_sherd", "D": "minecraft:danger_pottery_sherd", "B": "minecraft:brick"})
    except Exception as ex:
        F("craft helper:", repr(ex)[:160])
    time.sleep(0.5)
    F("inventory after crafting:", {k.split(":")[1]: v for k, v in a.inv().items() if "pot" in k or "sherd" in k or k.endswith("brick")})
    inv = a.state(0)["inv"]
    pots = [e for e in inv if e["id"] == "minecraft:decorated_pot"]
    F("pot components:", a.cmd(f"comps inv {pots[0]['slot']}")[:260] if pots else None)
    comps = a.cmd("comps inv 0") if False else None
    try:
        a.close()
    except Exception:
        pass


FN = {"insert": insert, "hopper": hopper, "break": breaking, "craft": craft}
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "mob_griefing true", "do_daylight_cycle false"):
    sv(f"gamerule {rule}", show=False)
sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=12)
for ph in PH:
    try:
        FN[ph]()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR in", ph, repr(ex)[:300])
kill_all()
