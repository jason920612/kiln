"""wp78 item 2b: the Wither's spawn explosion in lock step on both servers (same arena, same cows, same ticks).
Cows at fixed spots (free air and behind a wall) read their health right after the explosion tick (entity damage is deterministic: exposure rays);
the walls of 17 materials count survivors per material (block destruction uses random rays: compared as means over TRIALS).
usage: p78_wexp.py [TRIALS]"""
from lock import *
import collections, sys

TRIALS = int(sys.argv[1]) if len(sys.argv) > 1 else 5
CX, CY, CZ = 500, 200, 500
WALL = ["stone", "dirt", "oak_planks", "glass", "cobblestone", "obsidian", "netherrack", "sandstone", "stone_bricks", "iron_block", "bedrock", "end_stone", "deepslate", "gravel", "oak_log", "glowstone", "coal_block"]
COWS = [(3, 0), (0, 4), (-5, 0), (0, -3), (4, 4), (-5, -4), (0, -8), (8, 0), (0, 9), (-9, -3), (9, 9), (-3, -9), (11, 0), (0, 12)]
CL = {"K": 25999, "V": 26000}


def build(t):
    w(t, "kill @e[type=!player]")
    w(t, f"fill {CX - 24} {CY - 1} {CZ - 24} {CX + 24} {CY + 24} {CZ + 24} minecraft:air")
    w(t, f"fill {CX - 24} {CY - 1} {CZ - 24} {CX + 24} {CY - 1} {CZ + 24} minecraft:bedrock")
    for i, nm in enumerate(WALL):
        w(t, f"fill {CX - 8 + i} {CY} {CZ - 6} {CX - 8 + i} {CY + 3} {CZ - 6} minecraft:{nm}")
        w(t, f"fill {CX + 6} {CY} {CZ - 8 + i} {CX + 6} {CY + 3} {CZ - 8 + i} minecraft:{nm}")
    for i, (dx, dz) in enumerate(COWS):
        w(t, f"summon minecraft:cow {CX + dx}.5 {CY} {CZ + dz}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"c{i}\"]}}")


def wall_counts(t):
    c = C(CL[t])
    d = collections.Counter()
    for box in ((CX - 9, CY, CZ - 7, CX + 9, CY + 3, CZ - 5), (CX + 5, CY, CZ - 9, CX + 7, CY + 3, CZ + 9)):
        for l in json.loads(c.cmd("boxdump %d %d %d %d %d %d" % box)):
            x, y, z, st = l.split(" ", 3)
            d[st.split("[")[0].replace("minecraft:", "")] += 1
    return d


def num(t, txt):
    m = re.findall(r"(-?\d+(?:\.\d+)?)[fdb]?\]?\s*$", txt.strip().splitlines()[-1]) if txt.strip() else []
    return float(m[-1]) if m else None


def hp(t, sel):
    o = ask(t, f"data get entity {sel} Health", wait=0.2)
    m = re.findall(r"(-?\d+(?:\.\d+)?)f", o)
    return float(m[-1]) if m else None


def invul(t):
    o = ask(t, "data get entity @e[type=wither,limit=1] Invul", wait=0.2)
    m = re.findall(r"(-?\d+)\s*\]?\s*$", o.strip().splitlines()[-1]) if o.strip() else []
    return int(m[-1]) if m else None


def main():
    respawn_all()
    setup(frozen=False, rules=("mob_griefing true", "command_block_output false"))
    for t in "KV":
        w(t, "gamemode creative KilnTest")
        w(t, f"forceload add {CX - 30} {CZ - 30} {CX + 30} {CZ + 30}")
        w(t, f"tp KilnTest {CX + 20} {CY + 6} {CZ + 20}")
    time.sleep(8)
    for t in "KV":
        w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
    res = {t: [] for t in "KV"}
    for trial in range(TRIALS):
        both_("tick unfreeze")
        for t in "KV":
            build(t)
        time.sleep(1.5)
        both_("tick freeze")
        time.sleep(0.5)
        both_(f"summon minecraft:wither {CX}.5 {CY} {CZ}.5 {{Invul:220,Health:100f}}")
        time.sleep(0.5)
        before = {t: wall_counts(t) for t in "KV"}
        step(210)
        for t in "KV":
            w(t, "tick step 1")
        for k in range(14):
            iv = {t: invul(t) for t in "KV"}
            if all(v == 0 for v in iv.values()):
                break
            for t in "KV":
                if iv[t] != 0:
                    w(t, "tick step 1")
            time.sleep(0.25)
        time.sleep(0.4)
        cows = {t: [hp(t, f"@e[tag=c{i},limit=1]") for i in range(len(COWS))] for t in "KV"}
        after = {t: wall_counts(t) for t in "KV"}
        print(f"trial {trial}: invul {iv}", flush=True)
        print("  cows K", cows["K"], flush=True)
        print("  cows V", cows["V"], flush=True)
        print("  same cows:", cows["K"] == cows["V"], flush=True)
        for t in "KV":
            res[t].append(after[t])
            print(" ", t, "walls after:", dict(sorted(after[t].items())), flush=True)
    both_("tick unfreeze")
    print("== mean survivors per material over", TRIALS, "trials")
    mats = sorted(set(m for t in "KV" for r in res[t] for m in r))
    for m in mats:
        print(f"{m:14s} K {sum(r.get(m, 0) for r in res['K']) / TRIALS:6.1f}  V {sum(r.get(m, 0) for r in res['V']) / TRIALS:6.1f}")


main()
