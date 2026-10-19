"""wp80 item 4: crops growing by random ticks, Kiln against vanilla: N plants of a kind on moist farmland (or the soil they need) in the daylight, random tick speed
S, T ticks: how many are at each age afterwards. usage: p80_crops.py kind[,kind] [T] [S]"""
from lab80 import *
KINDS = {
    "torchflower_crop": ("minecraft:torchflower_crop[age={a}]", "farmland", 1),
    "pitcher_crop": ("minecraft:pitcher_crop[age={a},half=lower]", "farmland", 4),
    "wheat": ("minecraft:wheat[age={a}]", "farmland", 7),
    "beetroots": ("minecraft:beetroots[age={a}]", "farmland", 3),
    "carrots": ("minecraft:carrots[age={a}]", "farmland", 7),
    "sweet_berry_bush": ("minecraft:sweet_berry_bush[age={a}]", "grass_block", 3),
    "melon_stem": ("minecraft:melon_stem[age={a}]", "farmland", 7),
    "bamboo_sapling": ("minecraft:bamboo_sapling", "grass_block", 0),
    "sugar_cane": ("minecraft:sugar_cane[age={a}]", "sand", 15),
    "cactus": ("minecraft:cactus[age={a}]", "sand", 15),
    "nether_wart": ("minecraft:nether_wart[age={a}]", "soul_sand", 3),
    "kelp": ("minecraft:kelp[age={a}]", "sand", 25),
}
kinds = (sys.argv[1] if len(sys.argv) > 1 else "torchflower_crop,pitcher_crop,wheat,beetroots,sweet_berry_bush").split(",")
T = int(sys.argv[2]) if len(sys.argv) > 2 else 200
S = int(sys.argv[3]) if len(sys.argv) > 3 else 100
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false", "random_tick_speed 0"))
both_("time set 6000")
Y = 150
X0, Z0 = 4, 4
N = 40 * 20
for kind in kinds:
    tmpl, floor, maxage = KINDS[kind]
    both_("gamerule random_tick_speed 0")
    OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 41} {Y + 3} {Z0 + 21} minecraft:air")
    time.sleep(1.5)
    water = floor == "farmland"
    OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 40} {Y - 1} {Z0 + 20} minecraft:{floor}" + ("[moisture=7]" if water else ""))
    if kind == "kelp":
        OB(f"fill {X0} {Y} {Z0} {X0 + 39} {Y + 30} {Z0 + 19} minecraft:water")
    time.sleep(1.5)
    for z in range(0, 20):
        # one row of plants (age 0), in a fill: the whole row at once
        OB(f"fill {X0} {Y} {Z0 + z} {X0 + 39} {Y} {Z0 + z} {tmpl.format(a=0)}")
    time.sleep(1.0)
    both_(f"gamerule random_tick_speed {S}")
    time.sleep(T / 20.0 + 1.0)
    both_("gamerule random_tick_speed 0")
    time.sleep(0.5)
    res = {}
    for t in "KV":
        hist = []
        for a in range(0, maxage + 1):
            blk = tmpl.format(a=a)
            n = count_blocks(t, X0, Y, Z0, X0 + 39, Y, Z0 + 19, blk.split("[")[0] + (f"[age={a}]" if "{a}" in tmpl else ""))
            hist.append(n)
        res[t] = hist
    print(f"{kind:18s} (ticks {T}, speed {S}) K {res['K']}  V {res['V']}", flush=True)
OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 41} {Y + 31} {Z0 + 21} minecraft:air")
