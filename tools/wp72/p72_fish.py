"""wp72: fishing through a real client in an open pond (par.sh: the same script on Kiln and vanilla at once). A rod with Lure III and Luck of the Sea III,
N casts at tick rate 40; the caught items are classified (fish / junk / treasure). usage: p72_fish.py [N]"""
import sys, collections
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
N = int(sys.argv[1]) if len(sys.argv) > 1 else 30
ME = "KilnTest"
FISH = {"minecraft:cod", "minecraft:salmon", "minecraft:tropical_fish", "minecraft:pufferfish"}
TREASURE = {"minecraft:name_tag", "minecraft:saddle", "minecraft:bow", "minecraft:enchanted_book", "minecraft:nautilus_shell", "minecraft:fishing_rod"}

if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False)
sv("time set 6000", show=False)
sv("weather clear", show=False)
sv("gamerule do_daylight_cycle false", show=False)
platform(0, 0, 40, 40, y=199, clear=12)
# an open pond: 9x9, 3 deep, sunk into a stone slab (the platform is one layer: build a block of stone around it)
sv("fill 8 196 8 32 198 32 minecraft:stone", show=False)
sv("fill 12 197 12 20 198 20 minecraft:water", show=False)
sv("fill 12 199 12 20 199 20 minecraft:water", show=False)
sv(f"clear {ME}", show=False)
sv(f"gamemode survival {ME}", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
sv(f'give {ME} minecraft:fishing_rod[enchantments={{"minecraft:lure":3,"minecraft:luck_of_the_sea":3}}]', show=False)
sv(f"tp {ME} 16.5 200 24.5", show=False)
time.sleep(2.5)
p61.FISH_AT = (16.5, 199.5, 16.5)
res = p61.fish(a, N, rate=40, maxwait=40)
cnt = collections.Counter()
for hooked, d in res:
    for k, v in d.items():
        if v > 0 and k != "minecraft:fishing_rod":
            cnt["fish" if k in FISH else ("treasure" if k in TREASURE else "junk")] += 1
            cnt[k] += 1
print("RESULT", T, "casts", len(res), "hooked", sum(1 for h, d in res if h), dict(cnt), flush=True)
