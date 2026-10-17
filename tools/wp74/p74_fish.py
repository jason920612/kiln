"""wp74: fishing through a real client (par.sh: the same script on Kiln and vanilla at once, server tick rate raised so that N casts take minutes).
env POND=open|closed (a 9x9x3 pond / a 3x3x3 hole in stone, which is no open water), ENCH=none|lure3|luck3|both. usage: p74_fish.py [N] [RATE]"""
import sys, collections
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
N = int(sys.argv[1]) if len(sys.argv) > 1 else 100
RATE = int(sys.argv[2]) if len(sys.argv) > 2 else 100
POND = sys.argv[3] if len(sys.argv) > 3 else "open"
ENCH = sys.argv[4] if len(sys.argv) > 4 else "none"
ME = "KilnTest"
FISH = {"minecraft:cod", "minecraft:salmon", "minecraft:tropical_fish", "minecraft:pufferfish"}
TREASURE = {"minecraft:name_tag", "minecraft:saddle", "minecraft:bow", "minecraft:enchanted_book", "minecraft:nautilus_shell", "minecraft:fishing_rod"}

if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
for c in ("gamerule spawn_mobs false", "time set 6000", "weather clear", "gamerule advance_time false", "gamerule advance_weather false"):
    sv(c, show=False)
platform(0, 0, 40, 40, y=199, clear=12)
sv("fill 8 196 8 32 198 32 minecraft:stone", show=False)
if POND == "open":
    sv("fill 12 197 12 20 198 20 minecraft:water", show=False)
    sv("fill 12 199 12 20 199 20 minecraft:water", show=False)
else:
    sv("fill 15 197 15 17 198 17 minecraft:water", show=False)
    sv("fill 15 199 15 17 199 17 minecraft:water", show=False)
    # stone walls two blocks east and west of the bobber (inside its 5x4x5 open-water check), the south side stays open for the cast
    sv("fill 14 199 12 14 201 20 minecraft:stone", show=False)
    sv("fill 18 199 12 18 201 20 minecraft:stone", show=False)
sv(f"clear {ME}", show=False)
sv(f"gamemode survival {ME}", show=False)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
ench = {"none": "", "lure3": '[enchantments={"minecraft:lure":3}]', "luck3": '[enchantments={"minecraft:luck_of_the_sea":3}]',
        "both": '[enchantments={"minecraft:lure":3,"minecraft:luck_of_the_sea":3}]'}[ENCH]
sv(f"give {ME} minecraft:fishing_rod{ench}", show=False)
sv(f"tp {ME} 16.5 200 {21.5 if POND == 'closed' else 24.5}", show=False)
time.sleep(2.5)
p61.FISH_AT = (16.5, 199.5, 16.5)
t0 = time.time()
res = p61.fish(a, N, rate=RATE, maxwait=30)
if os.environ.get("DEBUG") or N <= 10:
    print("PERCAST", res, flush=True)
cnt = collections.Counter()
for hooked, d in res:
    for k, v in d.items():
        if v > 0 and k != "minecraft:fishing_rod" or (k == "minecraft:fishing_rod" and v > 0 and ENCH != "none" and False):
            cnt["fish" if k in FISH else ("treasure" if k in TREASURE else "junk")] += v
            cnt[k] += v
print("RESULT", T, POND, ENCH, "casts", len(res), "hooked", sum(1 for h, d in res if h), "secs", round(time.time() - t0), dict(cnt), flush=True)
