"""wp80 item 4: sniffer eggs (hatch time on moss and on sand, the three cracks) and sniffers digging (what they find, how often), Kiln against vanilla with `tick sprint`.
usage: p80_sniffer.py eggs|dig [ticks]"""
from lab80 import *
mode = sys.argv[1] if len(sys.argv) > 1 else "eggs"
T = int(sys.argv[2]) if len(sys.argv) > 2 else 9000
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false", "random_tick_speed 3"))
both_("difficulty normal")
both_("kill @e[type=!player]")
Y = 150
X0, Z0 = 6, 6
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 42} {Y + 10} {Z0 + 42} minecraft:air")
if mode == "eggs":
    OB(f"fill {X0} {Y - 1} {Z0} {X0 + 39} {Y - 1} {Z0 + 19} minecraft:sand")
    OB(f"fill {X0} {Y - 1} {Z0 + 20} {X0 + 39} {Y - 1} {Z0 + 39} minecraft:moss_block")
    for z in (Z0 + 2, Z0 + 22):
        for x in range(X0 + 1, X0 + 39, 2):
            OB(f"setblock {x} {Y} {z} minecraft:sniffer_egg[hatch=0]")
    time.sleep(1)
    for t in "KV":
        pass
    both_("tick freeze")
    time.sleep(0.5)
    # the sprint runs both servers (one after the other: each one's `tick sprint` returns when it is done)
    tot = 0
    marks = (3000, 6000, 9000, 12000, 16000)
    for m in marks:
        if m > T:
            break
        for t in "KV":
            sprint(t, m - tot)
        tot = m
        r = {}
        for t in "KV":
            r[t] = {
                "sand eggs left": count_blocks(t, X0, Y, Z0, X0 + 39, Y, Z0 + 19, "minecraft:sniffer_egg"),
                "moss eggs left": count_blocks(t, X0, Y, Z0 + 20, X0 + 39, Y, Z0 + 39, "minecraft:sniffer_egg"),
                "hatch2 (cracked twice)": count_blocks(t, X0, Y, Z0, X0 + 39, Y, Z0 + 39, "minecraft:sniffer_egg[hatch=2]"),
                "hatch1": count_blocks(t, X0, Y, Z0, X0 + 39, Y, Z0 + 39, "minecraft:sniffer_egg[hatch=1]"),
                "sniffers": count(t, f"@e[type=minecraft:sniffer,x={X0 - 2},y={Y - 2},z={Z0 - 2},dx=50,dy=20,dz=50]"),
            }
        print(f"after {m} ticks: K {r['K']}\n               V {r['V']}", flush=True)
    both_("tick unfreeze")
else:
    OB(f"fill {X0} {Y - 1} {Z0} {X0 + 39} {Y - 1} {Z0 + 39} minecraft:grass_block")
    OB(f"fill {X0 + 10} {Y - 1} {Z0 + 10} {X0 + 29} {Y - 1} {Z0 + 29} minecraft:moss_block")
    OB(f"fill {X0} {Y} {Z0} {X0 + 39} {Y + 4} {Z0} minecraft:barrier")
    OB(f"fill {X0} {Y} {Z0 + 39} {X0 + 39} {Y + 4} {Z0 + 39} minecraft:barrier")
    OB(f"fill {X0} {Y} {Z0} {X0} {Y + 4} {Z0 + 39} minecraft:barrier")
    OB(f"fill {X0 + 39} {Y} {Z0} {X0 + 39} {Y + 4} {Z0 + 39} minecraft:barrier")
    for i in range(10):
        OB(f"summon minecraft:sniffer {X0 + 12 + 3 * i % 15}.5 {Y} {Z0 + 12 + 2 * (i % 10)}.5 {{PersistenceRequired:1b}}")
    time.sleep(1)
    both_("tick freeze")
    time.sleep(0.5)
    tot = 0
    for m in (1500, 3000, 6000):
        if m > T:
            break
        for t in "KV":
            sprint(t, m - tot)
        tot = m
        r = {}
        for t in "KV":
            r[t] = {
                "seeds": count(t, f"@e[type=minecraft:item,x={X0},y={Y - 2},z={Z0},dx=40,dy=12,dz=40,nbt={{Item:{{id:\"minecraft:torchflower_seeds\"}}}}]"),
                "pods": count(t, f"@e[type=minecraft:item,x={X0},y={Y - 2},z={Z0},dx=40,dy=12,dz=40,nbt={{Item:{{id:\"minecraft:pitcher_pod\"}}}}]"),
                "items": count(t, f"@e[type=minecraft:item,x={X0},y={Y - 2},z={Z0},dx=40,dy=12,dz=40]"),
                "coarse/podzol-ish dirt?": count_blocks(t, X0, Y - 1, Z0, X0 + 39, Y - 1, Z0 + 39, "minecraft:dirt"),
            }
        print(f"after {m} ticks: K {r['K']}\n               V {r['V']}", flush=True)
    both_("tick unfreeze")
both_("kill @e[type=!player]")
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 42} {Y + 10} {Z0 + 42} minecraft:air")
