from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false"))
both_("time set 6000")
Y=150; x=4; z=4
OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 8} {Y + 12} {z + 6} minecraft:air")
OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 8} {Y - 1} {z + 6} minecraft:farmland[moisture=7]")
for age_l, age_u, dx in ((0, 0, 0), (3, 3, 2), (2, 2, 4), (0, 3, 6)):
    OB(f"setblock {x + dx} {Y} {z} minecraft:pitcher_crop[age={age_l},half=lower]")
    OB(f"setblock {x + dx} {Y + 1} {z} minecraft:pitcher_crop[age={age_u},half=upper]")
time.sleep(1.5)
for dx in (0, 2, 4, 6):
    for t in "KV":
        r = []
        for h in (0, 1):
            o = ask(t, f"execute in minecraft:overworld run execute if block {x+dx} {Y+h} {z} minecraft:pitcher_crop", wait=0.3)
            r.append("pitcher" if ("conditional.pass" in o or "Test passed" in o) else "-")
        print(dx, t, r, flush=True)
OB(f"fill {x - 1} {Y - 1} {z - 1} {x + 8} {Y + 12} {z + 6} minecraft:air")
