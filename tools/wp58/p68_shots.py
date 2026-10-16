"""wp68: scenes for the documentation's screenshots (set up with commands; the screenshot itself is taken from the Xvfb display by the caller)."""
from p68_small import *
import sys

scenario("shots")


def common():
    prep()
    sv("gamerule send_command_feedback false", show=False)
    time.sleep(2)
    sv("kill @e[type=!player]", show=False)
    sv("fill 10 200 10 34 215 38 minecraft:air", show=False)
    sv("time set day", show=False)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)


def shelf():
    common()
    for i, x in enumerate((19, 20, 21)):
        sv(f"setblock {x} 200 24 minecraft:oak_shelf[facing=north]", show=False)
    sv(f"item replace entity {ME} hotbar.0 with minecraft:diamond 3", show=False)
    sv(f"tp {ME} 20.5 200 21.5", show=False)
    time.sleep(1)
    for n, (x, it) in enumerate(((19.5, "diamond 3"), (20.5, "golden_apple 2"), (21.5, "enchanted_book 1"))):
        sv(f"item replace block {int(x)} 200 24 container.{n} with minecraft:{it}", show=False)
    sv("setblock 20 200 25 minecraft:redstone_block", show=False)
    a.lookat(20.5, 200.7, 24.5)
    time.sleep(1.5)


def ghast():
    common()
    sv("fill 10 199 10 34 199 38 minecraft:grass_block", show=False)
    sv("summon minecraft:happy_ghast 20.5 202 26.5 {NoAI:1b,equipment:{body:{id:'minecraft:blue_harness',count:1}}}", show=False)
    sv(f"tp {ME} 20.5 200 14.5", show=False)
    time.sleep(1.5)
    a.lookat(20.5, 203.5, 26.5)
    time.sleep(1.5)


def golem():
    common()
    sv("fill 10 199 10 34 199 38 minecraft:grass_block", show=False)
    sv("setblock 18 200 24 minecraft:copper_chest[facing=south]", show=False)
    sv("setblock 22 200 24 minecraft:chest[facing=south]", show=False)
    sv("summon minecraft:copper_golem 20.5 200 25.5 {weather_state:'oxidized'}", show=False)
    sv("summon minecraft:copper_golem 23.5 200 27.5 {}", show=False)
    time.sleep(1.5)
    a.lookat(20.5, 201, 25.5)
    time.sleep(1.5)


def sulfur():
    common()
    sv("fill 10 199 10 34 199 38 minecraft:grass_block", show=False)
    sv("summon minecraft:sulfur_cube 19.5 200 24.5 {NoAI:1b,Size:2}", show=False)
    sv("summon minecraft:mannequin 22.5 200 24.5 {immovable:1b}", show=False)
    time.sleep(1.5)
    a.lookat(21, 201, 24.5)
    time.sleep(1.5)


def creaking():
    common()
    sv("fill 10 199 10 34 199 38 minecraft:grass_block", show=False)
    sv("setblock 20 201 25 minecraft:pale_oak_log[axis=y]", show=False)
    sv("setblock 20 203 25 minecraft:pale_oak_log[axis=y]", show=False)
    sv("setblock 20 202 25 minecraft:creaking_heart[axis=y,natural=true]", show=False)
    sv("time set midnight", show=False)
    sv("gamerule spawn_mobs true", show=False)
    time.sleep(6)
    c = ents_of(a, "creaking", 30)
    if c:
        a.lookat(c[0]["x"], c[0]["y"] + 1.2, c[0]["z"])
    else:
        a.lookat(20.5, 202.5, 25.5)
    time.sleep(1.5)


def vault():
    common()
    sv("fill 10 199 10 34 199 38 minecraft:stone_bricks", show=False)
    sv("setblock 20 200 24 minecraft:vault[ominous=true,facing=south]{config:{key_item:{id:\"minecraft:ominous_trial_key\",count:1},loot_table:\"minecraft:chests/trial_chambers/reward_ominous\"}}", show=False)
    give(ME, "minecraft:ominous_trial_key", 1)
    a.select("minecraft:ominous_trial_key")
    sv(f"tp {ME} 20.5 200 21.5", show=False)
    time.sleep(1)
    a.lookat(20.5, 200.5, 24.5)
    time.sleep(0.6)
    a.mouse("right", "tap")
    time.sleep(1.6)


def bundle():
    common()
    sv(f"clear {ME}", show=False)
    sv(f"give {ME} minecraft:bundle[bundle_contents=[{{id:'minecraft:diamond',count:5}},{{id:'minecraft:stick',count:20}},{{id:'minecraft:apple',count:3}}]]", show=False)
    time.sleep(0.5)
    a.key("e", "tap")
    time.sleep(1.2)
    a.cmd("guihover 36")
    time.sleep(0.8)


if __name__ == "__main__":
    globals()[sys.argv[1]]()
    time.sleep(0.5)
