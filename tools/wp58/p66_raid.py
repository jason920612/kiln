"""wp66 item 2: a whole raid won by the player's own kills, on Kiln (TARGET=K) and on the vanilla reference (TARGET=V), same seed and village.
The actor (a real client) drinks an Ominous Bottle in the village, rings the bell when the first raiders are there, then fights every raider with a sword
(real clicks aimed at the entity; the script only moves the player next to the raider it picked, with /tp, as the walk across country took too long).
No /kill. It logs what the server holds every 1.5 s (raiders by wave and type, health, glow, vex and fangs around) and what the client shows (the boss bar's
name and progress), until Hero of the Village. After it: the effect, the villagers' gifts, the trade prices with and without the hero discount,
the fireworks of the celebration (watched by a separate process).
usage: TARGET=K|V [ACTOR=KilnTest AGENT_PORT=25999 SMALL=1] [DIFFICULTY=normal] [OMEN=2] python3 p66_raid.py OUT.json"""
import json, math, os, re, sys, time
from p66 import *
import p66rt as R

DIFFICULTY = os.environ.get("DIFFICULTY", "normal")
OMEN = int(os.environ.get("OMEN", "2"))
VX, VZ = int(os.environ.get("VX", "1463")), int(os.environ.get("VZ", "-1043"))
BOX = f"x={VX-150},y=-64,z={VZ-150},dx=300,dy=400,dz=300"
RAIDER_T = ["pillager", "vindicator", "ravager", "evoker", "witch", "vex", "illusioner"]
MODE = os.environ.get("MODE", "victory")   # victory: villagers protected, the player kills every wave; loss: all villagers killed once the first wave is there; timeout: a long sprint
NAME = os.environ.get("ACTOR", "KilnTest")
PORT = int(os.environ.get("AGENT_PORT", "25999"))
LOG = {"target": T, "difficulty": DIFFICULTY, "omen": OMEN, "events": [], "samples": []}
t_start = time.time()


def alive(c):
    """Respawns when dead (the small second window has its button elsewhere)."""
    if not os.environ.get("SMALL"):
        return p61.alive(c)
    for _ in range(4):
        if c.screen() != "DeathScreen":
            break
        c.cmd("mousepos 240 149"); time.sleep(1.5)
        c.mouse("left", "tap"); time.sleep(2.5)
    return c.state(0)["health"]


def F(*a):
    s = " ".join(str(x) for x in a)
    print(f"[{time.time()-t_start:6.0f}s] {s}", flush=True)
    LOG["events"].append((round(time.time() - t_start, 1), s))


def S(*cmds, timeout=60):
    return R.run(list(cmds), timeout=timeout)


def raiders_now():
    """[(type, wave, hp, glowing)] of every raider of the level (one query per raider type)."""
    rows = []
    for t in RAIDER_T:
        out = S(f"execute as @e[type=minecraft:{t},{BOX}] run data get entity @s")
        for l in out:
            if "entity data" not in l and "query[" not in l:
                continue
            w = re.search(r"Wave: (-?\d+)", l)
            hp = re.search(r"Health: ([\d.]+)f", l)
            g = "minecraft:glowing" in l
            rows.append((t, int(w.group(1)) if w else -1, float(hp.group(1)) if hp else 0.0, g))
    return rows


def villagers_now():
    out = S(f"execute as @e[type=minecraft:villager,{BOX}] run data get entity @s Health")
    return len([l for l in out if "has the following" in l or "query[" in l])


def bars(a):
    try:
        return json.loads(a.cmd("bossbars"))
    except Exception as e:
        return [f"ERR {e}"]


def effects(a):
    return {e["id"].split(":")[-1]: (e["amp"], e.get("ticks")) for e in a.state(0)["effects"]}


def prep(a):
    alive(a)
    S('datapack disable "file/wp66"')   # (its tick function removes iron golems)
    S("gamerule raids true", "gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false", f"difficulty {DIFFICULTY}", "time set 6000", "weather clear 1000000", "gamerule mob_griefing true")
    if T == "V":
        S(f"op {NAME}")
    S(f"forceload add {VX-64} {VZ-64} {VX+64} {VZ+64}")
    time.sleep(6)


def gear():
    n = NAME
    S(f"clear {n}", f"effect clear {n}", f"gamemode survival {n}")
    S(f'give {n} minecraft:netherite_sword[minecraft:enchantments={{"minecraft:sharpness":5,"minecraft:fire_aspect":2}}] 1',
      f"give {n} minecraft:golden_apple 32", f"give {n} minecraft:cooked_beef 64",
      f"give {n} minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier={OMEN-1}] 1")
    for slot, item in (("armor.head", "helmet"), ("armor.chest", "chestplate"), ("armor.legs", "leggings"), ("armor.feet", "boots")):
        S(f'item replace entity {n} {slot} with minecraft:netherite_{item}[minecraft:enchantments={{"minecraft:protection":4}}]')
    S(f"effect give {n} minecraft:resistance 99999 2 true", f"effect give {n} minecraft:strength 99999 1 true", f"effect give {n} minecraft:regeneration 99999 0 true")


def melee(a, deadline):
    """Real fighting: heal if low; pick the nearest raider (or vex); if it is far, /tp next to it; hit it until it is dead."""
    alive(a)
    if a.screen() is not None:
        a.cmd("key escape tap")
    st = a.state(0)
    if st["health"] < 9:
        a.select("minecraft:golden_apple")
        a.mouse("right", "down"); time.sleep(2.0); a.mouse("right", "up")
    a.select("minecraft:netherite_sword")
    es = [e for e in a.state(100)["entities"] if e["t"] in RAIDER_T]
    if not es:
        # raiders the client does not see (far, or down a hole): the server says where they are
        for t in RAIDER_T:
            for l in S(f"execute as @e[type=minecraft:{t},{BOX}] unless data entity @s {{Wave:-1}} run data get entity @s Pos"):
                m = re.findall(r"(-?\d+\.?\d*)d", l)
                if len(m) == 3 and ("entity data" in l or "query[" in l):
                    S(f"tp {NAME} {float(m[0])-1.5:.2f} {float(m[1]):.2f} {float(m[2]):.2f}")
                    time.sleep(1.0)
                    return 0
        return 0
    e = min(es, key=lambda e: (e["x"] - st["x"]) ** 2 + (e["z"] - st["z"]) ** 2 + (e["y"] - st["y"]) ** 2)
    d = math.dist((e["x"], e["y"], e["z"]), (st["x"], st["y"], st["z"]))
    if d > 4.5:
        S(f"tp {NAME} {e['x']-1.6:.2f} {e['y']:.2f} {e['z']:.2f}")
        time.sleep(0.4)
    t0 = time.time()
    while time.time() - t0 < min(10, deadline - time.time()):
        cur = next((x for x in a.state(40)["entities"] if x["id"] == e["id"]), None)
        if cur is None:
            return 1
        st = a.state(0)
        if st["health"] <= 0:
            return 0
        if math.dist((cur["x"], cur["y"], cur["z"]), (st["x"], st["y"], st["z"])) > 3.2:
            S(f"tp {NAME} {cur['x']-1.4:.2f} {cur['y']:.2f} {cur['z']:.2f}")
            time.sleep(0.25)
        a.lookat(cur["x"], cur["y"] + 0.9, cur["z"])
        time.sleep(0.12)
        a.mouse("left", "tap")
        time.sleep(0.55)
    # no progress: a pit of powder snow (the village was in a snowy place) lets the swings hit the snow, not the raider
    st = a.state(0)
    S(f"fill {int(st['x'])-5} {int(st['y'])-2} {int(st['z'])-5} {int(st['x'])+5} {int(st['y'])+3} {int(st['z'])+5} minecraft:air replace minecraft:powder_snow")
    return 0


def misc_around(a):
    """Counts of client-visible entities that tell what the raiders do (vexes, evoker fangs, thrown potions, arrows, fireworks)."""
    c = {}
    for e in a.state(80)["entities"]:
        if e["t"] in ("vex", "evoker_fangs", "potion", "splash_potion", "arrow", "firework_rocket", "ravager", "iron_golem", "item"):
            c[e["t"]] = c.get(e["t"], 0) + 1
    return c


def ring_bell(a):
    bells = a.find("bell", 20, 3)
    if not bells:
        return None
    bx, by, bz = bells[0]
    S(f"tp {NAME} {bx}.5 {by} {bz+2}.5")
    time.sleep(0.8)
    a.lookat(bx + 0.5, by + 0.5, bz + 0.5)
    time.sleep(0.3)
    a.mouse("right", "tap")
    time.sleep(1.2)
    rows = raiders_now()
    rows = [r for r in rows if r[1] >= 1]
    return {"raiders": len(rows), "glowing": sum(1 for r in rows if r[3])}


def break_all_pois():
    """Breaks every point of interest of the village's kinds that `/locate poi` finds around the bell (looping), so that the village is gone."""
    kinds = ["home", "meeting_point", "farmer", "librarian", "cleric", "armorer", "butcher", "cartographer", "fisherman", "fletcher", "leatherworker", "mason", "shepherd", "toolsmith", "weaponsmith"]
    broken = []
    for kind in kinds:
        for _ in range(60):
            out = S(f"execute positioned {VX} 111 {VZ} run locate poi minecraft:{kind}")
            line = next((l for l in out if "[" in l and ("success" in l or "nearest" in l.lower())), None)
            m = re.search(r"\[(-?\d+), (-?\d+), (-?\d+)\]", line or "")
            if not m:
                break
            p = tuple(int(x) for x in m.groups())
            S(f"setblock {p[0]} {p[1]} {p[2]} minecraft:air destroy")
            broken.append((kind, p))
    return broken


def run_raid(a, max_s=3000):
    prep(a)
    gear()
    S(f"tp {NAME} {VX} 130 {VZ}")
    time.sleep(6)
    g = a.ground(VX, VZ, 160) or 100
    S(f"tp {NAME} {VX}.5 {g} {VZ}.5")
    time.sleep(3)
    bells = a.find("bell", 120, 5)
    F("village: villagers", villagers_now(), "bells", bells[:3], "at", [round(x, 1) for x in (a.state(0)["x"], a.state(0)["y"], a.state(0)["z"])])
    if bells:
        bx, by, bz = bells[0]
        S(f"tp {NAME} {bx}.5 {by+1} {bz+2}.5")
        time.sleep(3)
    F("client bars before:", bars(a))
    a.select("minecraft:ominous_bottle")
    a.mouse("right", "down"); time.sleep(2.4); a.mouse("right", "up")
    time.sleep(0.5)
    F("effects after the bottle", effects(a))
    t0 = time.time()
    first = None
    last_wave_key = None
    last_bar_names = None
    kills = 0
    last_sample = 0
    quiet = 0
    protected_at = 0
    S("gamerule advance_time true", "tick rate 20")
    while time.time() - t0 < max_s:
        ef = effects(a)
        if "hero_of_the_village" in ef:
            F("hero of the village:", ef["hero_of_the_village"], "after", round(time.time() - t0), "s; kills", kills)
            LOG["hero"] = True
            break
        if MODE == "victory" and time.time() - protected_at > 45:
            # the village's own people take the blows of 40 raiders (the player is away fighting): resistance 5 makes them immune
            protected_at = time.time()
            S(f"effect give @e[type=minecraft:villager,{BOX}] minecraft:resistance infinite 4 true")
        if time.time() - last_sample >= 1.5:
            last_sample = time.time()
            rs = raiders_now()
            by_wave = {}
            for t, w, hp, gl in rs:
                by_wave.setdefault(w, {}).setdefault(t, [0, 0, 0.0])
                by_wave[w][t][0] += 1
                by_wave[w][t][1] += gl
                by_wave[w][t][2] += hp
            sample = {"t": round(time.time() - t0, 1), "waves": {str(w): {t: [v[0], v[1], round(v[2])] for t, v in d.items()} for w, d in by_wave.items()}, "bars": bars(a), "misc": misc_around(a),
                      "villagers": villagers_now() if len(LOG["samples"]) % 10 == 0 else None}
            LOG["samples"].append(sample)
            if any(r[1] >= 1 for r in rs) and first is None:
                first = time.time() - t0
                F("first raiders after", round(first), "s:", sample["waves"], "bar", sample["bars"])
                # the bell: raiders within 48 blocks glow
                F("bell rung:", ring_bell(a))
                if MODE == "loss":
                    F("villagers before the massacre:", villagers_now())
                    S(f"kill @e[type=minecraft:villager,{BOX}]")
                    F("points of interest broken:", break_all_pois())
                    F("villagers after:", villagers_now())
            key = tuple(sorted(by_wave))
            names = tuple(b.get("name") if isinstance(b, dict) else str(b) for b in sample["bars"])
            if key != last_wave_key or names != last_bar_names:
                last_wave_key, last_bar_names = key, names
                F("raiders by wave:", sample["waves"], "| bars:", sample["bars"], "| kills", kills)
            # the raid is over (bar gone, nobody left) for a while: stop watching
            quiet = quiet + 1 if first is not None and not sample["bars"] and not any(w >= 1 for w in by_wave) else 0
            if quiet >= 25:
                F("the raid is over (no bar, no raiders) after", round(time.time() - t0), "s; kills", kills, "villagers", villagers_now())
                break
        if a.state(0)["health"] <= 0:
            alive(a)
        if MODE == "loss":
            time.sleep(1.0)   # (the player only watches)
        else:
            kills += melee(a, t0 + max_s)
        if first is None and time.time() - t0 > 240:
            F("no raid after 240 s; effects", effects(a))
            break
    LOG["kills"] = kills
    return LOG.get("hero", False)


def talk(a, vid, tries=6):
    """Looks at the villager (the client is slow: waits for the crosshair) and uses it until the trading screen opens."""
    for _ in range(tries):
        cur = next((x for x in a.state(30)["entities"] if x["id"] == vid), None)
        if cur is None:
            return False
        st = a.state(0)
        if math.dist((cur["x"], cur["y"], cur["z"]), (st["x"], st["y"], st["z"])) > 3.0:
            S(f"tp {NAME} {cur['x']-1.4:.2f} {cur['y']:.2f} {cur['z']:.2f}")
            time.sleep(0.8)
        a.lookat(cur["x"], cur["y"] + 1.0, cur["z"])
        time.sleep(1.0)
        a.mouse("right", "tap")
        time.sleep(1.2)
        if a.screen() == "MerchantScreen":
            return True
    return False


def trade_prices(a):
    """The prices the client shows at a profession villager: with the hero effect, and again without it."""
    prof_pos = []
    for prof in ("librarian", "farmer", "cleric", "fisherman", "shepherd", "butcher", "toolsmith", "armorer", "weaponsmith", "mason", "cartographer", "leatherworker", "fletcher"):
        out = S(f'execute as @e[type=minecraft:villager,{BOX},nbt={{VillagerData:{{profession:"minecraft:{prof}"}}}}] run data get entity @s Pos')
        for l in out:
            m = re.findall(r"(-?\d+\.?\d*)d", l)
            if len(m) == 3 and ("entity data" in l or "query[" in l):
                prof_pos.append((prof, [float(x) for x in m]))
    F("professions in the village:", [p for p, _ in prof_pos])
    res = {}
    chosen = None
    for label in ("with the hero effect", "without it"):
        if label == "without it":
            S(f"effect clear {NAME} minecraft:hero_of_the_village")
            time.sleep(1.0)
        got = None
        for prof, pos in prof_pos:
            if chosen is not None and prof != chosen[0]:
                continue
            S(f"tp {NAME} {pos[0]-1.4:.2f} {pos[1]:.2f} {pos[2]:.2f}")
            time.sleep(2.0)
            near = [e for e in a.state(12)["entities"] if e["t"] == "villager" and e.get("prof") == prof]
            if not near:
                continue
            vid = near[0]["id"]
            if chosen is not None and vid != chosen[1]:
                continue
            if talk(a, vid):
                try:
                    got = json.loads(a.cmd("offers"))
                except Exception as ex:
                    got = str(ex)
                a.cmd("key escape tap")
                time.sleep(0.6)
                if got and got != []:
                    chosen = (prof, vid)
                    break
            if a.screen() is not None:
                a.cmd("key escape tap")
        res[label] = got
        F("offers", label, "villager", chosen, str(got)[:900])
    LOG["trades"] = res


def inventory(a):
    inv = {}
    for s in a.state(0)["inv"]:
        inv[s["id"].split(":")[-1]] = inv.get(s["id"].split(":")[-1], 0) + s["n"]
    return inv


def after_victory(a, secs=160):
    """Stays with the villagers: the hero effect, the gifts that arrive in the inventory (what the villagers throw to the hero in the day), the fireworks and
    what the villagers do (celebrating) as the client sees them."""
    inv0 = inventory(a)
    t0 = time.time()
    seen_fw = 0
    while time.time() - t0 < secs:
        m = misc_around(a)
        if m.get("firework_rocket"):
            seen_fw += 1
        time.sleep(1.0)
        if int(time.time() - t0) % 20 == 0:
            F("after victory:", round(time.time() - t0), "s bars", bars(a), "effects", effects(a), "firework samples", seen_fw)
    inv1 = inventory(a)
    gifts = {k: inv1.get(k, 0) - inv0.get(k, 0) for k in set(inv0) | set(inv1) if inv1.get(k, 0) != inv0.get(k, 0)}
    F("inventory changes while waiting (the villagers' gifts):", gifts)
    LOG["gifts"] = gifts
    LOG["firework_samples"] = seen_fw


if __name__ == "__main__":
    outp = sys.argv[1]
    a = client(PORT)
    try:
        ok = run_raid(a)
        F("end: effects", effects(a), "bars", bars(a), "villagers", villagers_now())
        if ok:
            after_victory(a)
            try:
                trade_prices(a)
            except Exception as ex:
                F("trade check failed:", ex)
    finally:
        json.dump(LOG, open(outp, "w"), indent=1)
        F("log written", outp)
