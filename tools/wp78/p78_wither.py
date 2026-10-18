"""wp78 item 2: build the Wither in survival (real block placement from soul sand and skulls), watch the charge-up and the spawn explosion
against a wall of test blocks, then fight it. par.sh: Kiln and vanilla at once, one real client each. Prints a timeline."""
import sys, collections
from p72 import *
import p61

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
CX, CY, CZ = int(os.environ.get("CX", "500")), 200, int(os.environ.get("CZ", "500"))
WALL = ["stone", "dirt", "oak_planks", "glass", "cobblestone", "obsidian", "netherrack", "sandstone", "stone_bricks", "iron_block", "bedrock", "end_stone", "deepslate", "gravel", "oak_log", "glowstone", "coal_block"]


def say(*x):
    print(*x, flush=True)


def nbt(sel, path):
    return nbt_of(sel, path, wait=0.4)


def arena():
    p61.platform(CX - 24, CZ - 24, CX + 24, CZ + 24, y=199, clear=24)
    for (x0, y0, z0, x1, y1, z1) in ((-20, 199, -20, 20, 199, 20), (-20, 224, -20, 20, 224, 20), (-20, 199, -20, -20, 224, 20), (20, 199, -20, 20, 224, 20), (-20, 199, -20, 20, 224, -20), (-20, 199, 20, 20, 224, 20)):
        fill(CX + x0, y0, CZ + z0, CX + x1, y1, CZ + z1, "minecraft:bedrock")
    kill_all()
    # wall of test blocks, 17 columns wide, 4 tall, at 6 blocks north (z - 6) and 6 blocks east
    for i, nm in enumerate(WALL):
        x = CX - 8 + i
        for y in range(CY, CY + 4):
            sb(x, y, CZ - 6, "minecraft:" + nm)
    # a second wall east (x + 6) in the same order
    for i, nm in enumerate(WALL):
        z = CZ - 8 + i
        for y in range(CY, CY + 4):
            sb(CX + 6, y, z, "minecraft:" + nm)


def wall_counts():
    """Surviving blocks per material in the two walls, as the client sees them (boxdump)."""
    d = collections.Counter()
    for box in ((CX - 9, CY, CZ - 7, CX + 9, CY + 3, CZ - 5), (CX + 5, CY, CZ - 9, CX + 7, CY + 3, CZ + 9)):
        for l in json.loads(a.cmd("boxdump %d %d %d %d %d %d" % box)):
            x, y, z, st = l.split(" ", 3)
            d[st.split("[")[0].replace("minecraft:", "")] += 1
    return dict(sorted(d.items()))


def gear():
    sv(f"gamemode survival {ME}", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    for slot, it in (("head", "diamond_helmet"), ("chest", "diamond_chestplate"), ("legs", "diamond_leggings"), ("feet", "diamond_boots")):
        sv(f"item replace entity {ME} armor.{slot} with minecraft:{it}[enchantments={{\"minecraft:protection\":4}}]", show=False)
    for it in ("diamond_sword", "bow[enchantments={\"minecraft:power\":5}]", "arrow 128", "soul_sand 8", "wither_skeleton_skull 4", "golden_apple 16", "cooked_beef 64"):
        sv(f"give {ME} minecraft:{it}", show=False)
    sv(f"effect give {ME} minecraft:resistance 3000 3 true", show=False)
    sv(f"effect give {ME} minecraft:instant_health 1 10 true", show=False)


def wither_info():
    h = nbt("@e[type=wither,limit=1]", "Health")
    inv = nbt("@e[type=wither,limit=1]", "Invul")
    pos = nbt("@e[type=wither,limit=1]", "Pos")
    return h, inv, pos


def place_until(item, x, y, z, face, want, tries=6):
    for _ in range(tries):
        a.place(item, x, y, z, face)
        time.sleep(0.6)
        if want in blk(a, x, y + (1 if face == "up" else 0), z):
            return True
    return False


def main():
    if T == "V":
        sv("op KilnTest", show=False)
    alive_auto(a)
    sv("difficulty normal", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("gamerule mob_griefing true", show=False)
    sv("gamerule natural_health_regeneration true", show=False)
    sv("gamerule command_block_output false", show=False)
    sv(f"gamemode creative {ME}", show=False)
    sv(f"tp {ME} {CX}.5 210 {CZ - 2}.5", show=False)
    time.sleep(3)
    arena()
    sv(f"tp {ME} {CX}.5 200 {CZ + 3}.5", show=False)
    gear()
    time.sleep(2)
    say("walls before", wall_counts())
    # six cows for the wither to attack
    for i in range(6):
        sv(f"summon minecraft:cow {CX - 10 + 4 * i}.5 200 {CZ + 12}.5 {{PersistenceRequired:1b}}", show=False)
    # --- build, in survival, standing south of the T and placing against faces
    a.select("minecraft:soul_sand")
    spot = (CX, CY, CZ + 3)
    sv(f"tp {ME} {CX}.5 {CY} {CZ + 3}.5", show=False)
    time.sleep(1)
    a.place("minecraft:soul_sand", CX, 199, CZ, "up"); time.sleep(0.4)
    a.place("minecraft:soul_sand", CX, 200, CZ, "up"); time.sleep(0.4)
    sv(f"tp {ME} {CX - 2}.5 {CY} {CZ}.5", show=False); time.sleep(1.2)
    a.place("minecraft:soul_sand", CX, 201, CZ, "west"); time.sleep(0.4)
    sv(f"tp {ME} {CX + 2}.5 {CY} {CZ}.5", show=False); time.sleep(1.2)
    a.place("minecraft:soul_sand", CX, 201, CZ, "east"); time.sleep(0.4)
    sb(CX, CY, CZ + 3, "minecraft:stone")
    sv(f"tp {ME} {CX}.5 {CY + 1} {CZ + 3}.5", show=False); time.sleep(1.2)
    say("T:", blk(a, CX, 200, CZ), blk(a, CX, 201, CZ), blk(a, CX - 1, 201, CZ), blk(a, CX + 1, 201, CZ))
    place_until("minecraft:wither_skeleton_skull", CX - 1, 201, CZ, "up", "skull")
    place_until("minecraft:wither_skeleton_skull", CX + 1, 201, CZ, "up", "skull")
    say("skulls:", blk(a, CX - 1, 202, CZ)[:40], blk(a, CX + 1, 202, CZ)[:40], "wither yet?", nbt("@e[type=wither,limit=1]", "Health"))
    t_place = time.time()
    a.place("minecraft:wither_skeleton_skull", CX, 201, CZ, "up")
    time.sleep(0.5)
    say("after last skull:", "T", blk(a, CX, 200, CZ), blk(a, CX, 201, CZ), "wither:", wither_info())
    # retreat 14 blocks south while it charges
    a.walk_to(CX + 0.5, CZ + 15.5, timeout=8, tol=1.5)
    # timeline of the charge-up
    for s in range(16):
        time.sleep(1)
        h, inv, pos = wither_info()
        bb = a.cmd("bossbars")
        say(f"t+{time.time() - t_place:4.1f}s hp={h} invul={inv} pos={pos[:60]} bossbar={bb[:120]}")
        if s >= 11 and inv.strip().startswith("0"):
            break
    time.sleep(1)
    say("walls after the spawn explosion", wall_counts())
    say("player health", a.state(0)["health"], "cows", count_ents("@e[type=cow]"))
    shot("wither_spawn_" + T)
    # --- fight: ranged while the wither is armoured (> half health), shield test at half, then melee
    t1 = time.time()
    tl = []
    effects_seen = set()
    skulls_max = 0
    shield_tests = []
    sv(f"effect give {ME} minecraft:strength 3000 1 true", show=False)
    while time.time() - t1 < int(os.environ.get("FIGHT", "180")):
        alive_auto(a)
        st = a.state(0)
        for ef in st.get("effects", []):
            effects_seen.add(ef if isinstance(ef, str) else str(ef.get("id", ef)))
        if st["health"] < 14:
            sv(f"effect give {ME} minecraft:instant_health 1 3 true", show=False)
        es = a.state(70)["entities"]
        ws = [e for e in es if e["t"] == "wither"]
        if not ws:
            break
        wi = ws[0]
        d = math.hypot(wi["x"] - st["x"], wi["z"] - st["z"])
        sk = [e for e in es if e["t"] in ("wither_skull", "wither_skull_dangerous")]
        skulls_max = max(skulls_max, len(sk))
        tl.append((round(time.time() - t1), round(wi["hp"]), round(d), round(st["health"], 1), len(sk)))
        if wi["hp"] > 150 and not shield_tests:
            pass
        if 100 < wi["hp"] <= 150 and len(shield_tests) < 2 and d < 40:
            # the half-health shield: arrows must not hurt
            h0 = float(re.findall(r"(\d+(?:\.\d+)?)f", nbt("@e[type=wither,limit=1]", "Health"))[-1])
            a.select("minecraft:bow")
            for _ in range(5):
                es2 = [e for e in a.state(70)["entities"] if e["t"] == "wither"]
                if es2:
                    a.shoot_ballistic(es2[0]["x"], es2[0]["y"] + 1.5, es2[0]["z"], charge=1.0)
                time.sleep(0.6)
            time.sleep(1.5)
            h1 = float(re.findall(r"(\d+(?:\.\d+)?)f", nbt("@e[type=wither,limit=1]", "Health"))[-1])
            shield_tests.append((h0, h1))
            continue
        if wi["hp"] > 150 or d > 14:
            a.select("minecraft:bow")
            a.shoot_ballistic(wi["x"], wi["y"] + 1.5, wi["z"], charge=1.0)
            time.sleep(0.4)
        else:
            a.select("minecraft:diamond_sword")
            if d > 3:
                a.walk_to(wi["x"], wi["z"], timeout=1.5, tol=2.0)
            a.lookat(wi["x"], wi["y"] + 1.2, wi["z"])
            time.sleep(0.1)
            a.mouse("left", "tap")
            time.sleep(0.5)
    say("fight timeline (s, wither hp, dist, player hp, skulls in view):", tl[::5])
    say("effects the player showed:", sorted(effects_seen), "max skulls in view", skulls_max)
    say("half-health arrow shield test (hp before, hp after 5 arrows):", shield_tests)
    say("wither alive?", bool([e for e in a.state(80)["entities"] if e["t"] == "wither"]), "bar", a.cmd("bossbars")[:100], "fight took", round(time.time() - t1))
    time.sleep(4)
    stars = sv("execute as @e[type=item] run data get entity @s Item.id", show=False, wait=0.8)
    say("items on the ground:", collections.Counter(re.findall(r'"(minecraft:[a-z_]+)"', stars)))
    say("wither roses:", len(a.findbox("wither_rose", CX - 30, CY - 2, CZ - 30, CX + 30, CY + 12, CZ + 30, 20)), "cows left", count_ents("@e[type=cow]"))
    say("chat:", [l[:90] for l in p61.chat() if "Teleported" not in l and "Set own" not in l and "Gave" not in l][-12:])
    say("mspt", mspt()[:120])
    shot("wither_end_" + T)


main()
