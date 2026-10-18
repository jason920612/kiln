"""wp78 item 3: the Ender Dragon with two real players (KilnTest = agent 25999, Obs = agent 26000) on ONE server (TARGET=K or V).
Phases (STEPS env): crystals (arrows at free ones, a bar broken by hand at caged ones), fight (bow while it flies, sword while it perches, bottles for the
breath), death (the sequence, exit portal, egg, gateway, xp), respawn (four crystals on the portal)."""
import collections, sys
from p72 import *
import p61

A, B = C(25999), C(26000)
ME = ["KilnTest", "Obs"]
STEPS = os.environ.get("STEPS", "crystals,fight,death,respawn").split(",")
END = "minecraft:the_end"


def say(*x):
    print(*x, flush=True)


def E(cmd, wait=0.4):
    return sv("execute in minecraft:the_end run " + cmd, show=False, wait=wait)


def alive2():
    alive_auto(A)
    alive_b(B)


def gear(nm):
    sv(f"gamemode survival {nm}", show=False)
    sv(f"clear {nm}", show=False)
    sv(f"effect clear {nm}", show=False)
    for slot, it in (("head", "diamond_helmet"), ("chest", "diamond_chestplate"), ("legs", "diamond_leggings"), ("feet", "diamond_boots")):
        sv(f"item replace entity {nm} armor.{slot} with minecraft:{it}[enchantments={{\"minecraft:protection\":4}}]", show=False)
    for it in ("diamond_sword[enchantments={\"minecraft:sharpness\":5}]", "diamond_pickaxe", "bow[enchantments={\"minecraft:power\":5,\"minecraft:infinity\":1}]", "arrow 64", "glass_bottle 8", "golden_apple 16", "cooked_beef 64", "end_crystal 4", "ender_pearl 4"):
        sv(f"give {nm} minecraft:{it}", show=False)
    sv(f"effect give {nm} minecraft:resistance 3000 2 true", show=False)
    sv(f"effect give {nm} minecraft:instant_health 1 10 true", show=False)



def ready(c, nm):
    """After a death: back in the End with the gear (setup commands)."""
    try:
        st = c.state(0)
        if "the_end" not in st.get("dimension", ""):
            sv(f"gamemode survival {nm}", show=False)
            E(f"tp {nm} 0.5 70 -12.5")
            time.sleep(2.5)
        if not any(e["id"] == "minecraft:bow" for e in c.state(0)["inv"]):
            gear(nm)
            say(nm, "died and was equipped again")
    except Exception as ex:
        say("ready error", repr(ex)[:80])

def heal_all():
    for c, nm in ((A, ME[0]), (B, ME[1])):
        try:
            if c.state(0)["health"] < 12:
                sv(f"effect give {nm} minecraft:instant_health 1 3 true", show=False)
        except Exception:
            pass


def ground_y(cl, x, z, y0=110):
    for y in range(y0, 40, -1):
        b = cl.block(x, y, z)
        if "air" not in b.split("[")[0]:
            return y
    return None


def crystals(cl):
    return [e for e in cl.state(150)["entities"] if e["t"] == "end_crystal"]


def dragon(cl):
    d = [e for e in cl.state(250)["entities"] if e["t"] == "ender_dragon"]
    return d[0] if d else None


def dnbt(path):
    return nbt_of("@e[type=ender_dragon,limit=1]", path, wait=0.35)


def main():
    if T == "V":
        sv("op KilnTest", show=False); sv("op Obs", show=False)
    alive2()
    sv("difficulty normal", show=False)
    sv("gamerule mob_griefing true", show=False)
    sv("gamerule natural_health_regeneration true", show=False)
    for nm in ME:
        sv(f"gamemode creative {nm}", show=False)
        E(f"tp {nm} 0 100 -10")
    time.sleep(12)
    if os.environ.get("FULLHP"):
        E("data merge entity @e[type=ender_dragon,limit=1] {Health:200f}")
    say("bossbars", A.cmd("bossbars")[:200], "dragon", dragon(A) and (round(dragon(A)["x"]), round(dragon(A)["y"]), round(dragon(A)["z"])))
    cs = crystals(A)
    say("crystals", len(cs))
    cages = {}
    for c in cs:
        x, y, z = round(c["x"] - 0.5), round(c["y"]), round(c["z"] - 0.5)
        bars = A.findbox("iron_bars", x - 2, y - 2, z - 2, x + 2, y + 3, z + 2, 60)
        cages[(x, y, z)] = len(bars)
    say("crystals with bars around (count of bars):", cages)
    for nm in ME:
        gear(nm)
    E(f"tp KilnTest 0.5 70 -20.5")
    E(f"tp Obs 3.5 70 -20.5")
    time.sleep(6)
    say("players at", me(A), me(B), "ground", ground_y(A, 0, -20, 90))
    if "crystals" in STEPS:
        free = [p for p, n in cages.items() if n == 0]
        caged = [p for p, n in cages.items() if n > 0]
        say("free", len(free), "caged", len(caged))
        t0 = time.time()
        only = int(os.environ.get("ONLY", "99"))
        for p in free[:only]:
            alive2()
            heal_all()
            # a small stone platform level with the crystal, 7 blocks towards the centre (the arrows then fly flat over the pillar's top)
            d = math.hypot(p[0], p[2]) or 1
            sx, sz = round(p[0] - p[0] / d * 7), round(p[2] - p[2] / d * 7)
            E(f"fill {sx - 1} {p[1] - 1} {sz - 1} {sx + 1} {p[1] - 1} {sz + 1} minecraft:stone")
            for nm in ME:
                E(f"tp {nm} {sx + 0.5} {p[1]} {sz + 0.5}")
            time.sleep(2.5)
            for c in (A, B):
                c.select("minecraft:bow")
            for k in range(8):
                if not [q for q in crystals(A) if abs(q["x"] - 0.5 - p[0]) < 1 and abs(q["z"] - 0.5 - p[2]) < 1]:
                    break
                for c in (A, B):
                    c.shoot_ballistic(p[0] + 0.5, p[1] + 0.9, p[2] + 0.5, charge=1.3)
                time.sleep(1.2)
            gone = not [q for q in crystals(A) if abs(q["x"] - 0.5 - p[0]) < 1 and abs(q["z"] - 0.5 - p[2]) < 1]
            say("free crystal", p, "destroyed" if gone else "STILL THERE", "rounds", k + 1, "crystals left", len(crystals(A)))
        say("free crystals done in", round(time.time() - t0), "s; bar", A.cmd("bossbars")[:120], "dragon hp", dnbt("Health")[-40:])
        for p in caged[:only]:
            alive2()
            heal_all()
            bars = A.findbox("iron_bars", p[0] - 4, p[1] - 3, p[2] - 4, p[0] + 4, p[1] + 6, p[2] + 4, 400)
            xs = [b[0] for b in bars]; ys = [b[1] for b in bars]; zs = [b[2] for b in bars]
            box = (min(xs), min(ys), min(zs), max(xs), max(ys), max(zs))
            say("cage of", p, "bars", len(bars), "box", box)
            # the face towards the island centre along the longer axis
            if abs(p[0]) >= abs(p[2]):
                face = (-1 if p[0] > 0 else 1, 0)
            else:
                face = (0, -1 if p[2] > 0 else 1)
            sx = (box[0] - 3 if face[0] < 0 else box[3] + 3) if face[0] else p[0]
            sz = (box[2] - 3 if face[1] < 0 else box[5] + 3) if face[1] else p[2]
            sx, sz = (p[0] - 0 if face[0] == 0 else (box[3] + 1 if face[0] > 0 else box[0] - 1)), (p[2] if face[1] == 0 else (box[5] + 1 if face[1] > 0 else box[2] - 1))
            E(f"fill {sx - 1} {p[1] - 1} {sz - 1} {sx + 1} {p[1] - 1} {sz + 1} minecraft:stone")
            for nm in ME:
                E(f"tp {nm} {sx + 0.5} {p[1]} {sz + 0.5}")
            time.sleep(2.5)
            A.select("minecraft:diamond_pickaxe")
            # break the bars in the line from the stand point to the crystal (feet and head level)
            mined = 0
            for k in range(1, 8):
                x = sx - face[0] * k if face[0] else sx
                z = sz - face[1] * k if face[1] else sz
                for yy in (p[1], p[1] + 1):
                    b = A.block(x, yy, z)
                    if "iron_bars" in b:
                        A.near(x, yy, z, dist=3.0, timeout=3)
                        if A.mine(x, yy, z, timeout=8):
                            mined += 1
            say("bars mined by hand:", mined)
            for c in (A, B):
                c.select("minecraft:bow")
            for k in range(8):
                if not [q for q in crystals(A) if abs(q["x"] - 0.5 - p[0]) < 1 and abs(q["z"] - 0.5 - p[2]) < 1]:
                    break
                for c in (A, B):
                    c.shoot_ballistic(p[0] + 0.5, p[1] + 0.9, p[2] + 0.5, charge=1.3)
                time.sleep(1.2)
            gone = not [q for q in crystals(A) if abs(q["x"] - 0.5 - p[0]) < 1 and abs(q["z"] - 0.5 - p[2]) < 1]
            say("caged crystal", p, "destroyed" if gone else "STILL THERE", "rounds", k + 1, "crystals left", len(crystals(A)), "bar", A.cmd("bossbars")[:90])
    if "fight" in STEPS:
        say("== fight")
        for nm in ME:
            E(f"tp {nm} 0.5 70 -12.5")
        time.sleep(3)
        t0 = time.time()
        timeline = []
        bottles0 = (A.inv().get("minecraft:dragon_breath", 0), B.inv().get("minecraft:dragon_breath", 0))
        seen_phases = collections.Counter()
        last_log = 0
        perch_hits = 0
        hurried = False
        while time.time() - t0 < int(os.environ.get("FIGHT", "420")):
            if not hurried and time.time() - t0 > int(os.environ.get("FASTFWD", "60")):
                hurried = True
                E("data merge entity @e[type=ender_dragon,limit=1] {Health:36f}")
                say("(setup) dragon health set to 36 at", round(time.time() - t0), "s")
            alive2()
            ready(A, ME[0]); ready(B, ME[1])
            heal_all()
            d = dragon(A)
            if not d:
                break
            try:
                ph = int(re.findall(r"-?\d+", dnbt("DragonPhase"))[-1])
                hp = float(re.findall(r"(\d+(?:\.\d+)?)f", dnbt("Health"))[-1])
            except Exception:
                ph, hp = -1, -1
            seen_phases[ph] += 1
            if d.get("hp", 99) <= 1.5:
                say("the dragon is down to 1 health (dying) at", round(time.time() - t0), "s")
                break
            if time.time() - last_log > 5:
                last_log = time.time()
                timeline.append((round(time.time() - t0), ph, hp, round(d["x"]), round(d["y"]), round(d["z"])))
            perched = ph in (3, 5, 6, 7)
            for c, nm in ((A, ME[0]), (B, ME[1])):
                try:
                    st = c.state(0)
                    if st["health"] <= 0:
                        continue
                    dist = math.hypot(d["x"] - st["x"], d["z"] - st["z"])
                    clouds = [e for e in c.state(12)["entities"] if e["t"] == "area_effect_cloud"]
                    if perched and dist < 40:
                        c.select("minecraft:diamond_sword")
                        if dist > 3.2:
                            c.walk_to(d["x"], d["z"], timeout=2.0, tol=2.4)
                        c.lookat(d["x"], d["y"] + 1.2, d["z"])
                        time.sleep(0.08)
                        c.mouse("left", "tap")
                        perch_hits += 1
                    elif clouds and c.inv().get("minecraft:glass_bottle", 0) > 0:
                        cl = min(clouds, key=lambda e: math.hypot(e["x"] - st["x"], e["z"] - st["z"]))
                        c.select("minecraft:glass_bottle")
                        if math.hypot(cl["x"] - st["x"], cl["z"] - st["z"]) > 1.2:
                            c.walk_to(cl["x"], cl["z"], timeout=1.5, tol=0.8)
                        c.lookat(cl["x"], cl["y"], cl["z"])
                        c.mouse("right", "tap")
                    elif dist < 60:
                        c.select("minecraft:bow")
                        c.shoot_ballistic(d["x"], d["y"] + 1.0, d["z"], charge=1.1)
                except Exception as ex:
                    say("loop error", repr(ex)[:100])
            time.sleep(0.2)
        say("fight timeline (s, phase, hp, x, y, z):", timeline[::2])
        say("phases seen (ticks of the loop):", dict(seen_phases), "melee swings", perch_hits)
        say("took", round(time.time() - t0), "s; bar", A.cmd("bossbars")[:100])
        say("dragon_breath bottles gained:", A.inv().get("minecraft:dragon_breath", 0) - bottles0[0], B.inv().get("minecraft:dragon_breath", 0) - bottles0[1])
        shot("end_fight_" + T)
    if "death" in STEPS:
        say("== death sequence")
        xp_before = (A.state(0).get("xp") or 0) + (B.state(0).get("xp") or 0)
        t0 = time.time()
        last = None
        rows = []
        gone_at = None
        for s in range(240):
            es = A.state(150)["entities"]
            dr = [e for e in es if e["t"] == "ender_dragon"]
            cnt = dict(collections.Counter(e["t"] for e in es if e["t"] in ("experience_orb", "end_crystal", "area_effect_cloud", "item")))
            portal = len(A.findbox("end_portal", -6, 55, -6, 6, 75, 6, 50))
            egg = len(A.findbox("dragon_egg", -6, 55, -6, 6, 80, 6, 5))
            gw = len(A.findbox("end_gateway", -120, 50, -120, 120, 120, 120, 5))
            bar = A.cmd("bossbars")[:48]
            row = ("dragon", (round(dr[0]["x"]), round(dr[0]["y"]), round(dr[0]["z"])) if dr else None, cnt, "portal", portal, "egg", egg, "gateway", gw, "bar", bar)
            if dr is None or not dr:
                gone_at = gone_at or round(time.time() - t0, 1)
            if row != last:
                say(round(time.time() - t0, 1), *row)
            last = row
            if gone_at and time.time() - t0 > gone_at + 12 and portal:
                break
            time.sleep(0.5)
        # collect every orb: the players stand on the pile
        for _ in range(6):
            orbs = [e for e in A.state(150)["entities"] if e["t"] == "experience_orb"]
            if not orbs:
                break
            o = orbs[0]
            for nm in ME:
                E(f"tp {nm} {o['x']} {o['y']} {o['z']}")
            time.sleep(3)
        sa, sb = A.state(0), B.state(0)
        say("xp levels", sa.get("xpLevel"), sb.get("xpLevel"), "total points", sa.get("xp"), sb.get("xp"), "gained by the two", (sa.get("xp") or 0) + (sb.get("xp") or 0) - xp_before)
        say("advancement-ish chat:", [l[:80] for l in p61.chat() if "Free the End" in l or "advancement" in l.lower()][-3:])
        shot("end_death_" + T)
    if "respawn" in STEPS:
        say("== respawn the dragon with four crystals")
        alive2()
        ready(A, ME[0]); ready(B, ME[1])
        for c, nm in ((A, ME[0]), (B, ME[1])):
            sv(f"clear {nm} minecraft:end_crystal", show=False)
            sv(f"give {nm} minecraft:end_crystal 4", show=False)
        spots = {A: [(3, 0), (0, -3)], B: [(-3, 0), (0, 3)]}
        stand = {A: (6.5, 0.5), B: (-5.5, 0.5)}
        t_last = None
        for c, nm in ((A, ME[0]), (B, ME[1])):
            sx, sz = stand[c]
            E(f"tp {nm} {sx} 59 {sz}")
        time.sleep(3)
        placed = 0
        for c, nm in ((A, ME[0]), (B, ME[1])):
            for (x, z) in spots[c]:
                sx, sz = (x + (3.5 if x >= 0 else -3.5), z + 0.5) if z == 0 else (x + 0.5, z + (3.5 if z > 0 else -3.5))
                E(f"tp {nm} {sx} 59 {sz}")
                time.sleep(1.5)
                c.place("minecraft:end_crystal", x, 59, z, "up")
                time.sleep(1.2)
                placed += 1
                say(nm, "placed crystal on", (x, 59, z), "crystals now", len([e for e in c.state(40)["entities"] if e["t"] == "end_crystal"]))
        t0 = time.time()
        last = None
        for s in range(90):
            es = A.state(150)["entities"]
            cnt = dict(collections.Counter(e["t"] for e in es if e["t"] in ("ender_dragon", "end_crystal", "area_effect_cloud")))
            bars = A.cmd("bossbars")[:70]
            pn = len(A.findbox("end_portal", -3, 55, -3, 3, 65, 3, 30))
            ph = dnbt("DragonPhase")[-14:] if cnt.get("ender_dragon") else "-"
            row = (cnt, "portal", pn, "bar", bars, "phase", ph)
            if row != last:
                say(round(time.time() - t0, 1), *row)
            last = row
            if cnt.get("ender_dragon") and s > 40:
                break
            time.sleep(1.0)
        shot("end_respawn_" + T)
    shot("end_crystals_" + T)


main()
