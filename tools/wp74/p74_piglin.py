"""wp74: piglin / hoglin rules with a real client in the nether (par.sh: the same script on Kiln and vanilla at once).
Cases: bare player near 3 piglins; gold helmet; chest opened with a real click (gold armour on); gold block broken with a real mine; warped fungus and a respawn anchor next to a hoglin.
Each case prints, per second of the first 6, whether the piglins/hoglin target the player and the player's health."""
import sys, collections
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
NX, NY, NZ = 800, 70, 800


def N(cmd, show=False, wait=0.4):
    return sv("execute in minecraft:the_nether run " + cmd, show=show, wait=wait)


def say(*x):
    print(*x, flush=True)


def room():
    sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing true", show=False)
    sv(f"gamemode survival {ME}", show=False); sv("difficulty normal", show=False)
    N(f"forceload add {NX - 4} {NZ - 4} {NX + 30} {NZ + 30}")
    sv(f"execute in minecraft:the_nether run tp {ME} {NX + 12} {NY} {NZ + 12}", show=False)
    time.sleep(8)
    N(f"fill {NX - 1} {NY - 1} {NZ - 1} {NX + 26} {NY + 5} {NZ + 26} minecraft:air")
    N(f"fill {NX - 1} {NY - 1} {NZ - 1} {NX + 26} {NY + 5} {NZ + 26} minecraft:stone hollow")
    N(f"fill {NX} {NY - 1} {NZ} {NX + 25} {NY - 1} {NZ + 25} minecraft:netherrack")
    sv("kill @e[type=!player]", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 3 true", show=False)
    sv("gamerule natural_health_regeneration false", show=False)
    sv(f"tp {ME} {NX + 12}.5 {NY} {NZ + 12}.5", show=False)
    time.sleep(2)


def brain(sel):
    o = sv(f"data get entity {sel} Brain.memories", show=False, wait=0.5)
    return o


def watch(label, selector, secs=8):
    """Health of the player every second after it was set to 20: a mob that attacks takes it down."""
    alive_auto(a)
    N(f"fill {NX} {NY - 1} {NZ} {NX + 25} {NY - 1} {NZ + 25} minecraft:netherrack")
    sv(f"execute in minecraft:the_nether run tp {ME} {NX + 12}.5 {NY} {NZ + 12}.5", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 3 true", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
    time.sleep(1.5)
    say("   player at", me(a))
    rows = []
    for s in range(secs):
        rows.append(round(a.state(0)["health"], 1))
        time.sleep(1)
    say(label, "health per second:", rows, "-> damage", round(rows[0] - min(rows), 1))
    return rows


def fresh_piglins(n=3, sword=True):
    sv("kill @e[type=!player]", show=False)
    for i in range(n):
        sv(f"execute in minecraft:the_nether run summon minecraft:piglin {NX + 12 + i}.5 {NY} {NZ + 4}.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,HandItems:[{{id:\"minecraft:golden_sword\",count:1}},{{}}]}}", show=False)


def main():
    if T == "V":
        sv("op KilnTest", show=False)
    alive_auto(a)
    room()
    say("== 1. bare player, 3 piglins 8 blocks away")
    fresh_piglins()
    time.sleep(0.5)
    watch("bare", "@e[type=piglin,limit=1]")
    sv("kill @e[type=!player]", show=False)
    say("== 2. golden helmet")
    sv(f"item replace entity {ME} armor.head with minecraft:golden_helmet", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 3 true", show=False)
    fresh_piglins()
    time.sleep(0.5)
    watch("gold helmet", "@e[type=piglin,limit=1]")
    say("== 3. chest opened with a real click, gold helmet on")
    N(f"setblock {NX + 12} {NY} {NZ + 14} minecraft:chest[facing=north]")
    time.sleep(1)
    a.lookat(NX + 12.5, NY + 0.5, NZ + 14.5)
    time.sleep(0.6)
    for _ in range(3):
        a.mouse("right", "tap")
        time.sleep(1.2)
        if a.screen():
            break
    say("screen", a.screen(), "(the chest was opened if it is a container screen)")
    rows = []
    for s in range(8):
        rows.append(round(a.state(0)["health"], 1))
        time.sleep(1)
    say("after the chest (gold helmet on) health per second:", rows, "-> damage", round(rows[0] - min(rows), 1), "| piglin brain:", brain("@e[type=piglin,limit=1]")[-150:].replace(chr(10), " "))
    try:
        a.key("esc", "tap")
    except Exception:
        pass
    sv("kill @e[type=!player]", show=False)
    say("== 4. gold block broken (iron pickaxe), gold helmet on")
    sv(f"item replace entity {ME} armor.head with minecraft:golden_helmet", show=False)
    sv(f"give {ME} minecraft:iron_pickaxe", show=False)
    time.sleep(0.5)
    a.select("minecraft:iron_pickaxe")
    N(f"setblock {NX + 12} {NY} {NZ + 14} minecraft:gold_block")
    fresh_piglins()
    sv(f"tp {ME} {NX + 12}.5 {NY} {NZ + 12}.5", show=False)
    time.sleep(1)
    a.lookat(NX + 12.5, NY + 0.5, NZ + 14.5)
    time.sleep(0.5)
    a.mouse("left", "down")
    time.sleep(3.5)
    a.mouse("left", "up")
    say("gold block now:", blk(a, NX + 12, NY, NZ + 14))
    say("   piglin brain right after the block broke:", brain("@e[type=piglin,limit=1]")[-170:].replace(chr(10), " "), "| piglins:", sv("execute if entity @e[type=piglin]", show=False)[-60:].replace(chr(10), " "))
    watch("after gold block", "@e[type=piglin,limit=1]")
    sv("kill @e[type=!player]", show=False)
    say("== 5. hoglin and a warped fungus / respawn anchor")
    for label, block in (("none", "minecraft:air"), ("warped_fungus", "minecraft:warped_fungus"), ("respawn_anchor", "minecraft:respawn_anchor")):
        sv("kill @e[type=!player]", show=False)
        sv(f"execute in minecraft:the_nether run summon minecraft:hoglin {NX + 6}.5 {NY} {NZ + 12}.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b}}", show=False)
        N(f"setblock {NX + 9} {NY} {NZ + 12} {block}")
        sv(f"tp {ME} {NX + 14}.5 {NY} {NZ + 12}.5", show=False)
        watch(f"hoglin, {label}", "@e[type=hoglin,limit=1]", secs=8)
        N(f"setblock {NX + 9} {NY} {NZ + 12} minecraft:air")


main()
