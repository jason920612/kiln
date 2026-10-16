"""wp68 #6: persistence across a restart. `setup` builds a world state with real actions where it is cheap and commands for the rest,
`dump` writes what the server reports about it (data get ... for entities, block entities, the player; scoreboard; gamerules) to
~/wp68/persist/<tag>_<K|V>.json. Run `setup` and `dump before`, restart the server (wp68_kiln_keep.sh / wp68_vanilla_keep.sh) and the client,
run `dump after`, and diff with persist_cmp.py.  usage: p68_persist.py setup | dump TAG"""
from p68_small import *
import sys, json as _json

scenario("persist")
PDIR = HOME / "wp68" / "persist"

# --- the state ----------------------------------------------------------------------------------------------------------------
BLOCKS = [
    ("chest", "minecraft:chest[facing=south]{Items:[{Slot:0b,id:'minecraft:diamond',count:3},{Slot:13b,id:'minecraft:oak_log',count:64},{Slot:26b,id:'minecraft:written_book',count:1,components:{'minecraft:written_book_content':{title:{raw:'T'},author:'a',pages:[{raw:'p1'}]}}}],CustomName:'\"Stash\"'}"),
    ("furnace", "minecraft:furnace[facing=south,lit=false]{Items:[{Slot:0b,id:'minecraft:raw_iron',count:20},{Slot:1b,id:'minecraft:coal',count:5}]}"),
    ("brewing", "minecraft:brewing_stand{Items:[{Slot:0b,id:'minecraft:potion',count:1,components:{'minecraft:potion_contents':{potion:'minecraft:water'}}},{Slot:3b,id:'minecraft:nether_wart',count:5},{Slot:4b,id:'minecraft:blaze_powder',count:3}]}"),
    ("hopper", "minecraft:hopper[facing=down]{Items:[{Slot:2b,id:'minecraft:stone',count:7}],TransferCooldown:5}"),
    ("dispenser", "minecraft:dispenser[facing=up]{Items:[{Slot:4b,id:'minecraft:arrow',count:12}]}"),
    ("barrel", "minecraft:barrel[facing=up]{Items:[{Slot:0b,id:'minecraft:emerald',count:9}]}"),
    ("jukebox", "minecraft:jukebox{RecordItem:{id:'minecraft:music_disc_cat',count:1}}"),
    ("lectern", "minecraft:lectern[facing=south,has_book=true]{Book:{id:'minecraft:writable_book',count:1,components:{'minecraft:writable_book_content':{pages:[{raw:'hello'}]}}},Page:0}"),
    ("sign", "minecraft:oak_sign[rotation=4]{front_text:{messages:['\"one\"','\"two\"','\"\"','\"\"'],color:'red',has_glowing_text:1b},back_text:{messages:['\"b\"','\"\"','\"\"','\"\"']},is_waxed:1b}"),
    ("banner", "minecraft:white_banner[rotation=2]{patterns:[{pattern:'minecraft:creeper',color:'red'},{pattern:'minecraft:border',color:'black'}]}"),
    ("pot", "minecraft:decorated_pot[facing=south]{sherds:['minecraft:angler_pottery_sherd','minecraft:archer_pottery_sherd','minecraft:brick','minecraft:blade_pottery_sherd'],item:{id:'minecraft:diamond',count:2}}"),
    ("hive", "minecraft:bee_nest[facing=south]{bees:[{entity_data:{id:'minecraft:bee',Health:10f},min_ticks_in_hive:1000,ticks_in_hive:50}],flower_pos:[I;3,200,3]}"),
    ("cmd", "minecraft:command_block[facing=north,conditional=false]{Command:'say hi',auto:0b,LastOutput:'\"x\"',TrackOutput:1b,CustomName:'\"cb\"'}"),
    ("spawner", "minecraft:spawner{SpawnData:{entity:{id:'minecraft:zombie'}},Delay:100s,MaxNearbyEntities:6s,RequiredPlayerRange:16s,SpawnRange:4s}"),
    ("campfire", "minecraft:campfire[lit=true]{Items:[{Slot:0b,id:'minecraft:beef',count:1}],CookingTimes:[I;100,0,0,0],CookingTotalTimes:[I;600,0,0,0]}"),
    ("shulker", "minecraft:shulker_box[facing=up]{Items:[{Slot:0b,id:'minecraft:cobblestone',count:64}],CustomName:'\"Box\"'}"),
    ("bell", "minecraft:bell[facing=north,attachment=floor]"),
    ("copperchest", "minecraft:copper_chest[facing=south]{Items:[{Slot:0b,id:'minecraft:iron_ingot',count:5}]}"),
    ("beacon", "minecraft:beacon{Levels:0,primary_effect:'minecraft:speed'}"),
    ("shelf", "minecraft:oak_shelf[facing=south]{Items:[{Slot:0b,id:'minecraft:apple',count:1}]}"),
    ("comparator", "minecraft:comparator[facing=south]{OutputSignal:7}"),
]


def place_blocks():
    for n, (name, spec) in enumerate(BLOCKS):
        x, z = 4 + 2 * (n % 10), 4 + 2 * (n // 10)
        sv(f"setblock {x} 200 {z} {spec}", show=False, wait=0.15)


def setup():
    prep()
    time.sleep(2)
    sv("kill @e[tag=p68]", show=False)
    sv("kill @e[tag=p68b]", show=False)
    sv("kill @e[type=oak_boat]", show=False)
    platform(0, 0, 40, 40, y=199, clear=14)
    sv(f"gamemode survival {ME}", show=False)
    sv(f"tp {ME} 30.5 200 10.5", show=False)
    time.sleep(2)
    sv(f"clear {ME}", show=False)
    # inventory items with components
    for it in (
        "minecraft:diamond_sword[enchantments={'minecraft:sharpness':3,'minecraft:looting':2},custom_name='\"Excalibur\"',damage=12]",
        "minecraft:bundle[bundle_contents=[{id:'minecraft:stick',count:5},{id:'minecraft:diamond',count:2}]]",
        "minecraft:white_shulker_box[container=[{slot:0,item:{id:'minecraft:gold_ingot',count:7}}]]",
        "minecraft:potion[potion_contents={potion:'minecraft:strong_swiftness'}]",
        "minecraft:written_book[written_book_content={title:{raw:'Book'},author:'me',pages:[{raw:'p1'},{raw:'p2'}]}]",
        "minecraft:firework_rocket[fireworks={flight_duration:2,explosions:[{shape:'star',colors:[I;16711680],has_twinkle:1b}]}]",
        "minecraft:ender_chest",
        "minecraft:map",
        "minecraft:filled_map[map_id=0]",
    ):
        sv(f"give {ME} {it}", show=False)
    # a real ender chest use
    sv("setblock 36 200 12 minecraft:ender_chest[facing=south]", show=False)
    give(ME, "minecraft:netherite_ingot", 5)
    sv(f"tp {ME} 36.5 200 10.5", show=False)
    time.sleep(1.5)
    a.lookat(36.5, 200.5, 12.5)
    time.sleep(0.6)
    a.mouse("right", "tap")
    time.sleep(1.2)
    F("ender chest screen:", a.screen())
    if a.screen():
        st = a.state(0)
        nether = [s["i"] for s in st["slots"] if s["id"] == "minecraft:netherite_ingot"]
        if nether:
            a.guiclick(nether[0], "left", shift=True)
            time.sleep(0.4)
        a.key("esc", "tap")
        time.sleep(0.6)
    # experience, hunger, health
    sv(f"xp set {ME} 17 levels", show=False)
    sv(f"effect give {ME} minecraft:regeneration 5000 1 true", show=False)
    # mobs: a tame horse with saddle, armor and a donkey with a chest, a llama with a chest and decor, leashed cow, named villager
    sv("summon minecraft:horse 10.5 200 20.5 {Tame:1b,PersistenceRequired:1b,Variant:515,Temper:55,Tags:[\"p68\"],Owner:[I;-393238913,-1202048737,-1839933655,145653155],equipment:{saddle:{id:'minecraft:saddle',count:1},body:{id:'minecraft:iron_horse_armor',count:1}},Health:20f}", show=False)
    sv("summon minecraft:donkey 12.5 200 20.5 {Tame:1b,PersistenceRequired:1b,ChestedHorse:1b,Tags:[\"p68\"],Items:[{Slot:2b,id:'minecraft:bread',count:9},{Slot:5b,id:'minecraft:apple',count:3}]}", show=False)
    sv("summon minecraft:llama 14.5 200 20.5 {Tame:1b,PersistenceRequired:1b,ChestedHorse:1b,Strength:3,Variant:2,Tags:[\"p68\"],equipment:{body:{id:'minecraft:red_carpet',count:1}},Items:[{Slot:2b,id:'minecraft:hay_block',count:2}]}", show=False)
    sv("summon minecraft:cow 16.5 200 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],CustomName:'\"Bessie\"',NoAI:1b}", show=False)
    sv("setblock 18 200 20 minecraft:oak_fence", show=False)
    sv("summon minecraft:villager 20.5 200 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],VillagerData:{profession:'minecraft:librarian',level:3,type:'minecraft:plains'},Xp:60,NoAI:1b,Offers:{Recipes:[{buy:{id:'minecraft:paper',count:24},sell:{id:'minecraft:emerald',count:1},uses:2,maxUses:16,xp:2,priceMultiplier:0.05f,demand:3}]}}", show=False)
    sv("summon minecraft:happy_ghast 24.5 203 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],NoAI:1b,equipment:{body:{id:'minecraft:blue_harness',count:1}}}", show=False)
    sv("summon minecraft:copper_golem 26.5 200 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],weather_state:'exposed',next_weather_age:123456L}", show=False)
    sv("summon minecraft:wolf 28.5 200 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],Owner:[I;-393238913,-1202048737,-1839933655,145653155],CollarColor:3b,Sitting:1b,equipment:{body:{id:'minecraft:wolf_armor',count:1,components:{'minecraft:damage':5}}}}", show=False)
    sv("summon minecraft:sulfur_cube 30.5 200 20.5 {PersistenceRequired:1b,Tags:[\"p68\"],NoAI:1b,Size:1,equipment:{body:{id:'minecraft:tnt',count:1}}}", show=False)
    sv("summon minecraft:minecart 8.5 200 30.5 {Tags:[\"p68\"],Passengers:[{id:'minecraft:pig',PersistenceRequired:1b,Tags:[\"p68\"]}]}", show=False)
    sv("summon minecraft:chest_minecart 10.5 200 30.5 {Tags:[\"p68\"],Items:[{Slot:1b,id:'minecraft:coal',count:12}]}", show=False)
    sv("summon minecraft:item_frame 12 200 34 {Facing:3b,Tags:[\"p68\"],Item:{id:'minecraft:filled_map',count:1,components:{'minecraft:map_id':0}},ItemRotation:2b}", show=False)
    sv("summon minecraft:armor_stand 14.5 200 30.5 {Tags:[\"p68\"],ShowArms:1b,Pose:{Head:[10f,20f,0f]},equipment:{head:{id:'minecraft:iron_helmet',count:1}}}", show=False)
    sv("summon minecraft:mannequin 16.5 200 30.5 {Tags:[\"p68\"],immovable:1b,description:'\"desc\"'}", show=False)
    time.sleep(1.5)
    # real: lead the cow to the fence post
    give(ME, "minecraft:lead", 1)
    sv(f"tp {ME} 17.5 200 22.5", show=False)
    time.sleep(1.5)
    cow = ents_of(a, "cow", 20)
    if cow:
        use_on_entity(a, cow[0]["id"], "minecraft:lead", dy=0.5)
        time.sleep(0.6)
        a.lookat(18.5, 200.8, 20.5)
        time.sleep(0.5)
        a.mouse("right", "tap")
        time.sleep(0.8)
    F("cow leash:", re.findall(r"[Ll]eash: \{[^}]*\}", sv("data get entity @e[type=cow,tag=p68,limit=1]", show=False)))
    place_blocks()
    # scoreboard, team, gamerules, advancement, stats
    for c in (
        "scoreboard objectives add p68score dummy \"P68\"",
        "scoreboard objectives add p68jumps minecraft.custom:minecraft.jump",
        "scoreboard objectives setdisplay sidebar p68score",
        f"scoreboard players set {ME} p68score 42",
        "scoreboard players set #fake p68score 7",
        "team add p68team",
        "team modify p68team color gold",
        "team modify p68team friendlyFire false",
        f"team join p68team {ME}",
        "gamerule keep_inventory true",
        "gamerule random_tick_speed 9",
        "gamerule mob_griefing false",
        "gamerule spawn_mobs false",
        "gamerule advance_time false",
        "difficulty hard",
        "time set 6000",
        "weather rain 100000",
        f"advancement grant {ME} only minecraft:story/mine_stone",
        f"advancement grant {ME} only minecraft:adventure/summon_iron_golem",
        f"spawnpoint {ME} 5 200 5",
        "setworldspawn 3 200 3",
        "tag @e[type=cow] add p68",
        "bossbar add p68:bar \"Bar\"",
        "bossbar set p68:bar value 40",
        "bossbar set p68:bar visible false",
        "data modify storage p68:s v set value {a:1,b:[1,2,3],c:\"x\"}",
        "worldborder set 500",
        "worldborder center 10 10",
    ):
        sv(c, show=False, wait=0.2)
    # real: jump (statistics) ten times
    sv(f"tp {ME} 30.5 200 30.5", show=False)
    time.sleep(1.5)
    for _ in range(10):
        a.key("space", "tap")
        time.sleep(0.6)
    # a villager with a bed and a job site of his own (no NoAI): the Brain memories after the tick
    sv("fill 33 200 36 37 203 40 minecraft:air", show=False)
    sv("setblock 34 200 38 minecraft:composter", show=False)
    sv("setblock 36 200 38 minecraft:red_bed[facing=east,part=foot]", show=False)
    sv("setblock 37 200 38 minecraft:red_bed[facing=east,part=head]", show=False)
    sv("summon minecraft:villager 35.5 200 38.5 {Tags:[\"p68b\"],PersistenceRequired:1b,VillagerData:{profession:'minecraft:none',level:1,type:'minecraft:plains'}}", show=False)
    sv("tick sprint 400", show=False, wait=0.5)
    time.sleep(1)
    for _ in range(60):
        if "sprinting" not in sv("tick query", show=False, wait=0.3).lower():
            break
        time.sleep(1)
    F("villager2:", re.findall(r"profession: \"[^\"]*\"", sv("data get entity @e[type=villager,tag=p68b,limit=1]", show=False)))
    # a hopper clock: two hoppers pass one item back and forth, a comparator reads the first one into a lamp
    sv("fill 4 200 33 8 200 37 minecraft:air", show=False)
    sv("setblock 5 200 36 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:'minecraft:stone',count:1}]}", show=False)
    sv("setblock 6 200 36 minecraft:hopper[facing=west]", show=False)
    sv("setblock 5 200 35 minecraft:comparator[facing=south]", show=False)
    sv("setblock 5 200 34 minecraft:redstone_lamp", show=False)
    time.sleep(3)
    # the boat last: the player rides it at the dump
    give(ME, "minecraft:oak_boat", 1)
    sv(f"tp {ME} 14.5 200 40.5", show=False)
    time.sleep(1.5)
    a.select("minecraft:oak_boat")
    a.lookat(14.5, 199.99, 38.5)
    time.sleep(0.6)
    a.mouse("right", "tap")
    time.sleep(1.0)
    boats = ents_of(a, "oak_boat", 20)
    if boats:
        interact_ent(a, boats[0]["id"], button="right", tries=6, dy=0.3)
        time.sleep(1.0)
    F("riding a boat:", a.state(0)["vehicle"], pos(a))
    F("setup done: lamp", blk(a, 5, 200, 34))


PROBES = [
    ("player", f"data get entity {ME}", None),
    ("ender", f"data get entity {ME} EnderItems", None),
    ("inventory", f"data get entity {ME} Inventory", None),
    ("spawn", f"data get entity {ME} respawn", None),
    ("xp", f"data get entity {ME} XpLevel", None),
    ("horse", "data get entity @e[type=horse,tag=p68,limit=1]", None),
    ("donkey", "data get entity @e[type=donkey,tag=p68,limit=1]", None),
    ("llama", "data get entity @e[type=llama,tag=p68,limit=1]", None),
    ("cow", "data get entity @e[type=cow,tag=p68,limit=1]", None),
    ("villager", "data get entity @e[type=villager,tag=p68,limit=1]", None),
    ("villager2", "data get entity @e[type=villager,tag=p68b,limit=1]", None),
    ("ghast", "data get entity @e[type=happy_ghast,tag=p68,limit=1]", None),
    ("golem", "data get entity @e[type=copper_golem,tag=p68,limit=1]", None),
    ("wolf", "data get entity @e[type=wolf,tag=p68,limit=1]", None),
    ("sulfur", "data get entity @e[type=sulfur_cube,tag=p68,limit=1]", None),
    ("minecart", "data get entity @e[type=minecart,tag=p68,limit=1]", None),
    ("chest_minecart", "data get entity @e[type=chest_minecart,tag=p68,limit=1]", None),
    ("frame", "data get entity @e[type=item_frame,tag=p68,limit=1]", None),
    ("armor_stand", "data get entity @e[type=armor_stand,tag=p68,limit=1]", None),
    ("mannequin", "data get entity @e[type=mannequin,tag=p68,limit=1]", None),
    ("pig_in_cart", "data get entity @e[type=pig,tag=p68,limit=1]", None),
    ("boat", "data get entity @e[type=oak_boat,limit=1]", None),
    ("score", "scoreboard players list " + ME, "plain"),
    ("scores_fake", "scoreboard players list #fake", "plain"),
    ("objectives", "scoreboard objectives list", "plain"),
    ("teams", "team list", "plain"),
    ("team_info", "team list p68team", "plain"),
    ("sidebar", "scoreboard players get " + ME + " p68jumps", "plain"),
    ("rule_keep", "gamerule keep_inventory", "plain"),
    ("rule_rts", "gamerule random_tick_speed", "plain"),
    ("rule_grief", "gamerule mob_griefing", "plain"),
    ("difficulty", "difficulty", "plain"),
    ("time", "time query daytime", "plain"),
    ("weather", "weather query", "plain"),
    ("adv1", f"advancement test {ME} only minecraft:story/mine_stone", "plain"),
    ("adv2", f"advancement test {ME} only minecraft:adventure/summon_iron_golem", "plain"),
    ("adv3", f"advancement test {ME} only minecraft:story/smelt_iron", "plain"),
    ("bossbar", "bossbar get p68:bar value", "plain"),
    ("storage", "data get storage p68:s", None),
    ("border", "worldborder get", "plain"),
    ("worldspawn", "data get storage p68:none", "plain"),
]


def dump(tag):
    PDIR.mkdir(parents=True, exist_ok=True)
    res = {}
    for name, cmd, mode in PROBES:
        res[name] = sv(cmd, show=False, wait=0.6)[-6000:]
    for n, (name, spec) in enumerate(BLOCKS):
        x, z = 4 + 2 * (n % 10), 4 + 2 * (n // 10)
        res["block_" + name] = sv(f"data get block {x} 200 {z}", show=False, wait=0.5)[-3000:]
        res["state_" + name] = blk(a, x, 200, z) if a.state(0).get("inGame") else ""
    res["map0"] = sv("data get entity " + ME + " Inventory", show=False, wait=0.5)[-100:]
    # the clock: sample the lamp
    lamp = []
    for _ in range(40):
        out = sv("execute if block 5 200 34 minecraft:redstone_lamp[lit=true]", show=False, wait=0.25)
        lamp.append("pass" in out and "fail" not in out)
        time.sleep(0.4)
    res["clock_lamp_samples"] = "".join("1" if x else "0" for x in lamp)
    res["tick_info"] = sv("time query gametime", show=False)
    res["client"] = _json.dumps({"vehicle": a.state(0).get("vehicle"), "pos": pos(a), "health": a.state(0).get("health"), "xp": a.state(0).get("xpLevel"), "effects": a.state(0).get("effects")})
    (PDIR / f"{tag}_{T}.json").write_text(_json.dumps(res, indent=1))
    print("dumped", len(res), "entries to", PDIR / f"{tag}_{T}.json")


if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else "setup"
    if which == "setup":
        try:
            setup()
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F("ERROR", repr(ex)[:300])
    else:
        dump(sys.argv[2] if len(sys.argv) > 2 else "x")
