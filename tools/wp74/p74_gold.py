"""wp74: piglins and a broken gold block, creative and survival, a real client (par.sh)."""
import sys
exec(open("p74_piglin.py").read().split("def main():")[0])
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
room()
for mode in ("survival",):
    for blk in ("gold_block", "gold_ore", "stone"):
        alive_auto(a)
        sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
        sv(f"gamemode {mode} {ME}", show=False)
        sv(f"item replace entity {ME} armor.head with minecraft:golden_helmet", show=False)
        sv("kill @e[type=!player]", show=False)
        sv(f"give {ME} minecraft:iron_pickaxe", show=False)
        a.select("minecraft:iron_pickaxe")
        sv(f"execute in minecraft:the_nether run tp {ME} {NX + 12}.5 {NY} {NZ + 12}.5", show=False)
        N(f"setblock {NX + 12} {NY} {NZ + 14} minecraft:{blk}")
        fresh_piglins()
        time.sleep(4.0)
        a.lookat(NX + 12.5, NY + 0.5, NZ + 14.5)
        time.sleep(0.5)
        say("  aim:", a.state(0).get("hit"), "pos", me(a), "screen", a.screen())
        a.mouse("left", "down")
        time.sleep(0.4 if mode == "creative" else 3.0)
        a.mouse("left", "up")
        time.sleep(0.8)
        say(mode, blk, "block now:", blk and p61.blk(a, NX + 12, NY, NZ + 14), "piglin brain:", brain("@e[type=piglin,limit=1]")[-110:].replace(chr(10), " "))
