"""wp84 #7: a server running the data pack `kw84` (tools/wp84/packs, written for this test) played in survival with one real client against each server (par.sh).
Functions (#load, #tick, macros, return values), predicates, advancements (toasts, rewards: xp, loot, recipe, function), loot table overrides (pig, dirt), recipe overrides
(stick x7, a shapeless recipe, smelting), the enchantments (damage with a condition, post_attack poison, damage_protection, attributes, mob_experience), tags (the axe mines dirt,
glass is climbable, bamboo blocks are planks), a damage type.
usage: par.sh p84_dp.py [phases: fn,adv,loot,craft,ench,tags]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
PH = (sys.argv[1] if len(sys.argv) > 1 else "fn,adv,loot,craft,ench,tags").split(",")
scenario("dp84")


def num(txt):
    m = re.findall(r"(-?\d+(?:\.\d+)?)", txt)
    return m[-1] if m else txt[-60:]


def score(name):
    o = sv(f"scoreboard players get {name} kw84", show=False, wait=0.5)
    m = re.findall(r"has (-?\d+)|\[(-?\d+)\]|: (-?\d+)|, (-?\d+)\]", o)
    return [x for t in m for x in t if x][-1:] or o[-80:]


def reset():
    kill_all()
    kit(ME)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"attribute {ME} minecraft:max_health base set 200", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.2)


def fx(cnt, pre=("sound", "particle", "UpdateAdvancements", "SystemChat", "level_event", "entity_event", "UpdateRecipes", "RecipeBook", "Recipe", "SetExperience", "Container")):
    return sorted(k for k in cnt if k.startswith(pre))[:12]


def fn():
    F("loads", score("#loads"), "cycles", score("#cycles"))
    sv("function kw84:macro {who:\"KilnTest\",item:\"minecraft:emerald\",amount:3}", show=False, wait=0.8)
    F("macro: emeralds", a.inv().get("minecraft:emerald", 0), "#macro", score("#macro"))
    o = sv("execute store result score #r kw84 run function kw84:nested", show=False, wait=0.8)
    F("nested return: #r", score("#r"), "#inner", score("#inner"))
    sv(f"give {ME} minecraft:gold_ingot 4", show=False)
    a.select("minecraft:gold_ingot")
    time.sleep(0.5)
    o = sv(f"execute as {ME} if predicate kw84:holding_gold", show=False, wait=0.6)
    F("predicate holding_gold (holding gold):", "pass" in o.lower() or "passed" in o.lower())
    a.cmd("spy on"); a.cmd("spylog clear")
    s0 = score(ME)
    time.sleep(3.0)
    ls = json.loads(a.cmd("spylog clear"))
    parts = sum(1 for l in ls if l.startswith("particle") and "happy_villager" in l)
    F("tick function: sidebar score of the player grew by", score(ME), "from", s0, "(2 particles per tick, spy particle lines:", parts, ")")
    sv(f"item replace entity {ME} weapon.mainhand with minecraft:stick", show=False)
    o = sv(f"execute as {ME} if predicate kw84:holding_gold", show=False, wait=0.6)
    F("predicate holding_gold (holding a stick):", "pass" in o.lower() or "passed" in o.lower())
    sv("time set 14000", show=False)
    o = sv(f"execute as {ME} if predicate kw84:night", show=False, wait=0.6)
    F("predicate night at 14000:", "pass" in o.lower() or "passed" in o.lower())
    sv("time set day", show=False)


def adv():
    reset()
    sv(f"advancement revoke {ME} everything", show=False, wait=1.0)
    sv(f"experience set {ME} 0 points", show=False)
    a.cmd("spy on"); a.cmd("spylog clear")
    sv(f"give {ME} minecraft:gold_nugget 3", show=False, wait=0.8)
    time.sleep(1.5)
    ls = json.loads(a.cmd("spylog clear"))
    cnt = collections.Counter(re.sub(r"-?\d+(\.\d+)?", "#", l)[:90] for l in ls if l.split(" ")[0] in ("UpdateAdvancements", "SystemChat", "SetExperience", "UpdateRecipes", "RecipeBookAdd", "sound"))
    F("gold nuggets x3 -> inventory advancement: packets", dict(cnt))
    F("xp", sv(f"data get entity {ME} XpTotal", show=False)[-30:], "inventory", {k.split(":")[1]: v for k, v in a.inv().items() if k.split(":")[1] in ("emerald", "amethyst_shard", "gold_nugget")}, "#rewards", score("#rewards"))
    sv(f"give {ME} minecraft:bread 2", show=False)
    a.select("minecraft:bread")
    a.cmd("spylog clear")
    a.mouse("right", "down"); time.sleep(2.2); a.mouse("right", "up")
    time.sleep(1.0)
    ls = json.loads(a.cmd("spylog clear"))
    cnt = collections.Counter(re.sub(r"-?\d+(\.\d+)?", "#", l)[:90] for l in ls if l.split(" ")[0] in ("UpdateAdvancements", "SystemChat", "SetExperience"))
    F("eating bread (hidden challenge): packets", dict(cnt), "xp", sv(f"data get entity {ME} XpTotal", show=False)[-30:])
    for k in ("kw84:nuggets", "kw84:bread", "kw84:pig_kill", "kw84:either"):
        o = sv(f"advancement test {ME} only {k}", show=False, wait=0.5)
        F("test", k, "grant" if ("Success" in o or "success" in o) and "true" in o.lower() else o[-70:])
    sv(f"give {ME} minecraft:stick 1", show=False, wait=0.8)
    time.sleep(1.0)
    o = sv(f"advancement test {ME} only kw84:either", show=False, wait=0.5)
    F("either-or after a stick:", o[-70:])


def loot():
    reset()
    sv(f"give {ME} minecraft:diamond_sword", show=False)
    a.select("minecraft:diamond_sword")
    got = collections.Counter()
    for i in range(4):
        sv("summon minecraft:pig 20.5 200 24.5 {Tags:[\"p\"],PersistenceRequired:1b}", show=False)
        time.sleep(1.0)
        before = collections.Counter(e.get("item") for e in a.state(10)["entities"] if e["t"] == "item")
        kill_nearest(a, "pig", rng=12, timeout=15)
        time.sleep(1.0)
        for e in a.state(10)["entities"]:
            if e["t"] == "item":
                got[(e.get("item"), e.get("n", 1))] += 1
        sv("kill @e[type=item]", show=False)
    F("pig kills x4: item stacks dropped", sorted(got.items()))
    sv("fill 18 199 22 22 199 26 minecraft:dirt", show=False)
    sv(f"give {ME} minecraft:iron_shovel 1", show=False)
    a.select("minecraft:iron_shovel")
    dirt = collections.Counter()
    for x in range(18, 22):
        a.mine(x, 199, 24, timeout=10)
        time.sleep(0.5)
        for e in a.state(8)["entities"]:
            if e["t"] == "item":
                dirt[e.get("item")] += e.get("n", 1)
        sv("kill @e[type=item]", show=False)
    F("dirt mined with a shovel x4:", dict(dirt))


def craft():
    reset()
    sv(f"give {ME} minecraft:oak_planks 4", show=False)
    sv(f"give {ME} minecraft:bamboo_block 2", show=False)
    sv(f"give {ME} minecraft:dirt 4", show=False)
    sv(f"give {ME} minecraft:gold_nugget 1", show=False)
    time.sleep(0.5)
    try:
        a.craft2(["P", "P"], {"P": "minecraft:oak_planks"})
        F("oak planks x2 -> sticks:", a.inv().get("minecraft:stick", 0))
        a.craft2(["P", "P"], {"P": "minecraft:bamboo_block"})
        F("bamboo block x2 -> sticks (planks tag):", a.inv().get("minecraft:stick", 0))
        a.craft2(["DD", "N "], {"D": "minecraft:dirt", "N": "minecraft:gold_nugget"})
        F("dirt, dirt, nugget (shapeless, unlocked by the advancement) -> diamonds:", a.inv().get("minecraft:diamond", 0))
    except Exception as ex:
        F("craft error", repr(ex)[:160])
    a.close()
    sv("setblock 20 200 24 minecraft:furnace", show=False)
    sv('data merge block 20 200 24 {Items:[{Slot:0b,id:"minecraft:gravel",count:4},{Slot:1b,id:"minecraft:coal",count:2}]}', show=False)
    time.sleep(12.0)
    F("furnace after 12 s:", re.sub(r"\s+", " ", sv("data get block 20 200 24 Items", show=False))[-200:])


def ench():
    reset()
    sv("summon minecraft:zombie 20.5 200 24.5 {NoAI:1b,PersistenceRequired:1b,Tags:[\"z\"],attributes:[{id:\"minecraft:max_health\",base:100}],Health:100f}", show=False)
    time.sleep(1.0)
    ids = [e["id"] for e in a.state(10)["entities"] if e["t"] == "zombie"]
    for name, gear in (("plain", ""), ("venom2", '[enchantments={"kw84:venom":2}]')):
        sv("data merge entity @e[tag=z,limit=1] {Health:100f}", show=False)
        sv(f"item replace entity {ME} weapon.mainhand with minecraft:diamond_sword{gear}", show=False)
        a.select("minecraft:diamond_sword")
        time.sleep(0.6)
        h0 = nbt_of("@e[tag=z,limit=1]", "Health", 0.5)
        if ids:
            interact_ent(a, ids[0], button="left", tries=8, dy=1.0)
        time.sleep(1.0)
        h1 = nbt_of("@e[tag=z,limit=1]", "Health", 0.5)
        eff = re.findall(r'id: "minecraft:(\w+)"', nbt_of("@e[tag=z,limit=1]", "active_effects", 0.5))
        F("sword", name, "zombie health", num(h0), "->", num(h1), "effects", eff)
    sv(f"item replace entity {ME} armor.chest with minecraft:iron_chestplate[enchantments={{\"kw84:plating\":2}}]", show=False)
    time.sleep(0.6)
    h = [float(num(sv(f"data get entity {ME} Health", show=False, wait=0.4))) for _ in range(1)]
    sv(f"damage {ME} 10 minecraft:generic", show=False, wait=0.5)
    time.sleep(0.5)
    h2 = float(num(sv(f"data get entity {ME} Health", show=False, wait=0.4)))
    F("plating II + iron chestplate: 10 generic damage took", round(h[0] - h2, 2), "speed attribute", re.findall(r'movement_speed[^}]*', sv(f"data get entity {ME} attributes", show=False))[:1])
    sv(f"effect clear {ME} minecraft:resistance", show=False)
    sv(f"attribute {ME} minecraft:max_health base set 200", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
    time.sleep(0.5)
    h = float(num(sv(f"data get entity {ME} Health", show=False, wait=0.4)))
    sv(f"damage {ME} 10 minecraft:generic", show=False, wait=0.5)
    time.sleep(0.5)
    h2 = float(num(sv(f"data get entity {ME} Health", show=False, wait=0.4)))
    F("(resistance cleared) plating II + iron chestplate: 10 generic damage took", round(h - h2, 2))
    sv(f"damage {ME} 3 kw84:laser", show=False, wait=0.5)
    time.sleep(0.5)
    F("kw84:laser 3 damage: health now", num(sv(f"data get entity {ME} Health", show=False, wait=0.4)), "food exhaustion", num(sv(f"data get entity {ME} foodExhaustionLevel", show=False, wait=0.4)))


def tags():
    reset()
    sv("fill 18 200 24 22 206 24 minecraft:glass", show=False)
    sv("fill 18 199 20 22 199 26 minecraft:stone", show=False)
    sv(f"tp {ME} 20.5 200 23.0", show=False)
    time.sleep(1.0)
    a.look(0, 0)
    y0 = pos(a)[1]
    a.key("w", "down")
    time.sleep(2.5)
    a.key("w", "up")
    F("walking into a glass wall (climbable by the pack's tag): y", y0, "->", pos(a)[1])
    sv("fill 18 200 24 22 206 24 minecraft:air", show=False)
    sv("fill 18 199 20 22 199 26 minecraft:dirt", show=False)
    sv(f"give {ME} minecraft:iron_axe 1", show=False)
    a.select("minecraft:iron_axe")
    t0 = time.time()
    ok = a.mine(20, 199, 24, timeout=15)
    F("iron axe on dirt (mineable/axe adds dirt): broke", ok, "in", round(time.time() - t0, 2), "s")
    sv("setblock 20 199 24 minecraft:dirt", show=False)
    sv(f"give {ME} minecraft:iron_shovel 1", show=False)
    a.select("minecraft:iron_shovel")
    t0 = time.time()
    ok = a.mine(20, 199, 24, timeout=15)
    F("iron shovel on dirt: broke", ok, "in", round(time.time() - t0, 2), "s")


FN = {"fn": fn, "adv": adv, "loot": loot, "craft": craft, "ench": ench, "tags": tags}
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for rule in ("spawn_mobs false", "mob_griefing true", "do_daylight_cycle false", "natural_regeneration false", "send_command_feedback true"):
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
