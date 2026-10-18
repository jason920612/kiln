"""wp78 item 1a: the nether fortress on foot (par.sh: Kiln and vanilla at once, the same seed-78 fortress at (64,~,192)).
Survival player with armour, fire resistance and a bow; real chest opening, real blaze spawner fight, nether wart mining, mob census."""
import sys, collections
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
SMALL = int(os.environ.get("KILN_AGENT_PORT", "25999")) == 26000


def N(cmd, show=False, wait=0.4):
    return sv("execute in minecraft:the_nether run " + cmd, show=show, wait=wait)


def say(*x):
    print(*x, flush=True)


def gear():
    sv(f"gamemode survival {ME}", show=False)
    sv(f"clear {ME}", show=False)
    for slot, it in (("head", "diamond_helmet"), ("chest", "diamond_chestplate"), ("legs", "diamond_leggings"), ("feet", "diamond_boots"), ("offhand", "shield")):
        sv(f"item replace entity {ME} {'weapon.offhand' if slot == 'offhand' else 'armor.' + slot} with minecraft:{it}", show=False)
    for it in ("diamond_sword", "diamond_pickaxe", "bow", "cooked_beef 64", "arrow 64", "golden_apple 8"):
        sv(f"give {ME} minecraft:{it}", show=False)
    sv(f"effect clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:fire_resistance 3000 0 true", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
    sv(f"effect give {ME} minecraft:resistance 3000 1 true", show=False)


def go_to(x, y, z):
    sv(f"execute in minecraft:the_nether run tp {ME} {x} {y} {z}", show=False)
    time.sleep(3.5)


def census(rng=48):
    es = a.state(rng)["entities"]
    c = collections.Counter(e["t"] for e in es if e["t"] != "item")
    return dict(sorted(c.items()))


def main():
    if T == "V":
        sv("op KilnTest", show=False)
    alive_auto(a)
    sv("difficulty normal", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("gamerule natural_health_regeneration true", show=False)
    gear()
    sv(f"gamemode creative {ME}", show=False)
    go_to(64.5, 90, 190.5)
    time.sleep(5)
    spot = free_spot(a, 64, 83, 190, r=8, dy=8)
    say("start spot", spot)
    go_to(spot[1] + 0.5, spot[2], spot[3] + 0.5)
    sv(f"gamemode survival {ME}", show=False)
    time.sleep(4)
    st = a.state(0)
    say("start", me(a), "health", st["health"], "census", census())
    shot("fort_start_" + T)
    # --- chests (real opening), nearest first
    chests = a.find("chest", 60, 40)
    chests = sorted(chests, key=lambda b: (b[0] - 64) ** 2 + (b[2] - 192) ** 2)
    seen = set()
    say("chests found", len(chests), chests[:10])
    n_open = 0
    for b in ([] if os.environ.get("SKIPCHEST") else chests):
        if n_open >= 6:
            break
        pos = (b[0], b[1], b[2])
        if pos in seen:
            continue
        seen.add(pos)
        spot = free_spot(a, pos[0], pos[1], pos[2], r=2, dy=2)
        if not spot:
            say("chest", pos, "no spot"); continue
        sv(f"execute in minecraft:the_nether run tp {ME} {spot[1]+0.5} {spot[2]} {spot[3]+0.5}", show=False)
        time.sleep(3)
        a.close()
        a.use_on(*pos)
        time.sleep(1.2)
        st = a.state(0)
        items = sorted((s["id"].replace("minecraft:", ""), s["n"]) for s in st.get("slots", []) if s["i"] < 27 and s["n"] > 0)
        say("chest", pos, a.screen(), items)
        a.close()
        n_open += 1
    # --- nether wart
    warts = a.findbox("nether_wart", 40, 60, 160, 100, 100, 230, 10)
    say("nether_wart blocks", len(warts), warts[:4])
    # --- blaze spawner fight
    spawners = a.find("spawner", 70, 20)
    say("spawners", spawners)
    sp = spawners[0] if spawners else None
    tot = collections.Counter()
    if sp:
        spot = free_spot(a, sp[0], sp[1], sp[2], r=3, dy=3)
        say("spawner", sp, "standing at", spot)
        sv(f"execute in minecraft:the_nether run tp {ME} {spot[1]+0.5} {spot[2]} {spot[3]+0.5}", show=False)
        time.sleep(3)
        before = a.inv()
        kills = 0
        seen_types = collections.Counter()
        t1 = time.time()
        while time.time() - t1 < int(os.environ.get("FIGHT", "110")):
            alive_auto(a)
            if a.state(0)["health"] < 9:
                sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)
            for e in a.state(40)["entities"]:
                seen_types[e["t"]] += 0
            c = census(30)
            for k, v in c.items():
                seen_types[k] = max(seen_types[k], v)
            if c.get("blaze"):
                a.select("minecraft:diamond_sword")
                if kill_nearest(a, "blaze", rng=30, timeout=14, dy=0.6):
                    kills += 1
            else:
                time.sleep(1.5)
        after = a.inv()
        d = {k: after.get(k, 0) - before.get(k, 0) for k in set(after) | set(before) if after.get(k, 0) != before.get(k, 0)}
        say("spawner fight 110 s: blaze kills", kills, "inventory delta", d, "max simultaneous", dict(seen_types))
        shot("fort_spawner_" + T)
    say("mspt", mspt()[:120])
    say("chunks", chunks()[:160])


main()
