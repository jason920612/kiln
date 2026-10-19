"""wp83 (on the VM, real client connected to the server under test): what the client makes of the pack's content, as text.

usage: python3 p83_cmp.py K|V    (the same script against Kiln and vanilla; diff ~/wp83/out_K.txt ~/wp83/out_V.txt)

Gives items carrying the pack's registry entries, summons a painting and animals with its variants, shows its dialog and
plays its song, and prints what the client holds: `comps inv N` (the item with its component patch, holders resolved from the
client's synchronized registries), `entdata EID` (the synched entity data the client holds), the open screen and overlay.
Entity ids are left out of the lines so the files compare.
"""
import re
import sys
import time

from c83 import C, con, shot

side = sys.argv[1]
out = open(f"/home/test/wp83/out_{side}.txt", "w")


def line(*a):
    s = " ".join(str(x) for x in a)
    out.write(s + "\n")
    print(s[:300])


c = C()
con("gamemode survival KilnTest", side)
con("clear KilnTest", side)
con("kill @e[type=!player]", side)
gifts = [
    'minecraft:stick[minecraft:jukebox_playable="kiln83:tune"]',
    'minecraft:iron_chestplate[minecraft:trim={material:"kiln83:coal_blue",pattern:"kiln83:wave"}]',
    'minecraft:goat_horn[minecraft:instrument="kiln83:horn"]',
    'minecraft:iron_chestplate[minecraft:trim={material:"minecraft:iron",pattern:"minecraft:sentry"}]',
    'minecraft:white_banner[minecraft:banner_patterns=[{pattern:"kiln83:zigzag",color:"red"}]]',
    'minecraft:painting[minecraft:painting/variant="kiln83:speck"]',
]
for g in gifts:
    con(f"give KilnTest {g}", side, 0.3)
time.sleep(1)
for i in range(len(gifts)):
    line("slot", i, c.cmd(f"comps inv {i}"))

s = c.state()
x, y, z = int(s["x"]), int(s["y"]), int(s["z"])
summons = [
    ('minecraft:painting', f'{x+4} {y+1} {z-2}', '{variant:"kiln83:sunrise_wide",facing:2b}'),
    ('minecraft:painting', f'{x+6} {y+1} {z-2}', '{variant:"minecraft:aztec",facing:2b}'),
    ('minecraft:wolf', f'{x+2} {y} {z+3}', '{variant:"kiln83:azure",sound_variant:"kiln83:foxy"}'),
    ('minecraft:pig', f'{x-2} {y} {z+3}', '{variant:"kiln83:spotted",sound_variant:"kiln83:hisser"}'),
    ('minecraft:cow', f'{x-4} {y} {z+3}', '{variant:"kiln83:dun",sound_variant:"kiln83:bleater"}'),
    ('minecraft:cat', f'{x+4} {y} {z+3}', '{variant:"kiln83:calico"}'),
    ('minecraft:chicken', f'{x+6} {y} {z+3}', '{variant:"kiln83:speckled"}'),
    ('minecraft:frog', f'{x-6} {y} {z+3}', '{variant:"kiln83:dusk"}'),
]
for kind, pos, nbt in summons:
    con(f"summon {kind} {pos} {nbt}", side, 0.3)
time.sleep(2)
s = c.state(40)
seen = {}
for e in s["entities"]:
    if e["t"] in ("painting", "wolf", "pig", "cow", "cat", "chicken", "frog"):
        seen.setdefault(e["t"], []).append(e["id"])
for kind in ("painting", "wolf", "pig", "cow", "cat", "chicken", "frog"):
    for eid in seen.get(kind, []):
        data = c.cmd(f"entdata {eid}")
        # Health and other values that vary are dropped: keep the registry references.
        refs = re.findall(r"Reference\{ResourceKey\[[^\]]*\]=[^\n]*", data)
        line(kind, "entdata", data if not refs else data)

con("dialog show KilnTest kiln83:hello", side, 2)
line("dialog screen", c.state()["screen"])
shot(f"/home/test/wp83/shots/dialog_{side}.png")
c.key("esc")
time.sleep(0.5)
con(f"setblock {x+1} {y} {z} minecraft:jukebox", side)
time.sleep(0.5)
c.cmd("hotbar 1")
c.lookat(x + 1.5, y + 0.5, z + 0.5)
time.sleep(0.5)
c.mouse("right")
time.sleep(1.5)
line("jukebox overlay", c.state()["overlay"])
shot(f"/home/test/wp83/shots/jukebox_{side}.png")

# The pack's `hoe` block transformer (it replaces vanilla's rules): stone with air above becomes diamond (3 damage), gravel becomes sand
# and drops the pack's flint out of the clicked face; grass does not till.
con("clear KilnTest minecraft:stick", side)
con("give KilnTest minecraft:diamond_hoe", side)
time.sleep(0.5)
slot = next(i for i in range(9) if "diamond_hoe" in c.cmd(f"item {i}"))
c.cmd(f"hotbar {slot + 1}")
tx, tz = x - 1, z + 2
for dx, kind in enumerate(("stone", "gravel", "grass_block")):
    con(f"setblock {tx + dx} {y - 1} {tz} minecraft:stone", side, 0.1)
    con(f"setblock {tx + dx} {y} {tz} minecraft:{kind}", side, 0.1)
time.sleep(1)
for dx, kind in enumerate(("stone", "gravel", "grass_block")):
    c.lookat(tx + dx + 0.5, y + 1.0, tz + 0.5)
    time.sleep(0.4)
    c.mouse("right")
    time.sleep(1.2)
    line("hoe on", kind, "->", c.block(tx + dx, y, tz))
    line("hoe item", c.cmd(f"comps inv {slot}"))
items = [e for e in c.state(40)["entities"] if e["t"] == "item"]
line("item entities near", len(items))
out.close()
