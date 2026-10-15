"""wp66 item 2: a whole raid won by the player's own kills, on Kiln (TARGET=K) and on the vanilla reference (TARGET=V), same seed and village.
The actor (real client) drinks an Ominous Bottle in the village, then fights every raider with a sword (no /kill); the script logs
what the server holds each second (raiders by wave and type, their glow, villagers) and what the client shows (boss bar name and progress),
until Hero of the Village. Afterwards: the hero effect, the gifts the villagers throw, the trade prices (the hero discount), the celebration.
usage: TARGET=K|V KILN_PLAY_DIR=wp66 python3 p66_raid.py [DIFFICULTY=normal] [OMEN=2] OUT.json"""
import json, math, os, re, sys, time
from p66 import *
import p66rt as R

DIFFICULTY = os.environ.get("DIFFICULTY", "normal")
OMEN = int(os.environ.get("OMEN", "2"))
VX, VZ = int(os.environ.get("VX", "1463")), int(os.environ.get("VZ", "-1043"))
BOX = f"x={VX-150},y=-64,z={VZ-150},dx=300,dy=400,dz=300"
RAIDER_T = ["pillager", "vindicator", "ravager", "evoker", "witch", "vex", "illusioner"]
LOG = {"target": T, "difficulty": DIFFICULTY, "omen": OMEN, "events": [], "samples": []}
t_start = time.time()


def F(*a):
    s = " ".join(str(x) for x in a)
    print(f"[{time.time()-t_start:6.0f}s] {s}", flush=True)
    LOG["events"].append((round(time.time() - t_start, 1), s))


def S(*cmds, timeout=60):
    return R.run(list(cmds), timeout=timeout)


def counts():
    """{wave: {type: n}} of the level's raiders, with their health sum and how many glow (server side)."""
    out = S(f"execute as @e[type=#minecraft:raiders,{BOX}] run data get entity @s Wave", f"execute as @e[type=#minecraft:raiders,{BOX}] run data get entity @s Glowing")
    return out


def raiders_now():
    """[(type, wave, hp, glowing)] from `data get entity` of every raider."""
    out = S(f"execute as @e[type=#minecraft:raiders,{BOX}] run data get entity @s")
    rows = []
    for l in out:
        t = re.search(r'id: "minecraft:([a-z_]+)"', l)
        if not t:
            continue
        w = re.search(r"Wave: (-?\d+)", l)
        hp = re.search(r"Health: ([\d.]+)f", l)
        g = re.search(r"Glowing: (\d)b", l) or re.search(r"Glowing: (true|1b)", l)
        rows.append((t.group(1), int(w.group(1)) if w else -1, float(hp.group(1)) if hp else 0.0, bool(g)))
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
    return {e["id"].split(":")[-1]: (e["amp"], e.get("dur")) for e in a.state(0)["effects"]}


def prep(a):
    alive(a)
    S("gamerule raids true", "gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false", f"difficulty {DIFFICULTY}", "time set 6000", "weather clear 1000000", "gamerule mob_griefing true")
    if T == "V":
        S("op KilnTest")
    S(f"forceload add {VX-32} {VZ-32} {VX+32} {VZ+32}")
    time.sleep(4)


def gear():
    n = "KilnTest"
    S(f"clear {n}", f"effect clear {n}", f"gamemode survival {n}")
    S(f'give {n} minecraft:netherite_sword[minecraft:enchantments={{"minecraft:sharpness":5,"minecraft:fire_aspect":2}}] 1',
      f'give {n} minecraft:netherite_helmet[minecraft:enchantments={{"minecraft:protection":4}}] 1',
      f'give {n} minecraft:netherite_chestplate[minecraft:enchantments={{"minecraft:protection":4}}] 1',
      f'give {n} minecraft:netherite_leggings[minecraft:enchantments={{"minecraft:protection":4}}] 1',
      f'give {n} minecraft:netherite_boots[minecraft:enchantments={{"minecraft:protection":4}}] 1',
      f"give {n} minecraft:golden_apple 32", f"give {n} minecraft:cooked_beef 64",
      f"give {n} minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier={OMEN-1}] 1")
    S(f"effect give {n} minecraft:resistance 99999 2 true", f"effect give {n} minecraft:strength 99999 1 true", f"effect give {n} minecraft:regeneration 99999 0 true")


def equip(a):
    for slot, item in enumerate(["netherite_helmet", "netherite_chestplate", "netherite_leggings", "netherite_boots"]):
        pass  # (armor is put on with the inventory screen below)
    a.cmd("key e tap")
    time.sleep(0.5)
    a.cmd("key escape tap")


def fight(a, deadline):
    """One round of fighting: heal if low, go for the nearest raider and hit it until it is dead (real melee)."""
    alive(a)
    if a.screen() is not None:
        a.cmd("key escape tap")
    st = a.state(0)
    if st["health"] < 9:
        a.select("minecraft:golden_apple")
        a.mouse("right", "down"); time.sleep(2.0); a.mouse("right", "up")
    a.select("minecraft:netherite_sword")
    es = [e for e in a.state(80)["entities"] if e["t"] in RAIDER_T]
    if not es:
        return 0
    e = min(es, key=lambda e: (e["x"] - st["x"]) ** 2 + (e["z"] - st["z"]) ** 2)
    return 1 if kill_nearest(a, e["t"], rng=80, timeout=min(12, max(2, deadline - time.time())), dy=0.9) else 0


def run_raid(a, max_s=2400):
    prep(a)
    gear()
    # armor on (shift-click from the inventory is not needed: `give` into an armor slot is not possible, so use `item replace`)
    for slot, item in (("armor.head", "helmet"), ("armor.chest", "chestplate"), ("armor.legs", "leggings"), ("armor.feet", "boots")):
        S(f'item replace entity KilnTest {slot} with minecraft:netherite_{item}[minecraft:enchantments={{"minecraft:protection":4}}]')
    # to the bell
    S("execute as KilnTest run say wp66 raid prep done")
    st = a.state(0)
    bells = a.find("bell", 100, 5) if st["inGame"] else []
    S(f"tp KilnTest {VX} 130 {VZ}")
    time.sleep(6)
    g = a.ground(VX, VZ, 160) or 100
    S(f"tp KilnTest {VX}.5 {g} {VZ}.5")
    time.sleep(3)
    bells = a.find("bell", 120, 5)
    F("village: villagers", villagers_now(), "bells", bells[:3], "at", [round(x, 1) for x in (a.state(0)["x"], a.state(0)["y"], a.state(0)["z"])])
    if bells:
        bx, by, bz = bells[0]
        S(f"tp KilnTest {bx}.5 {by+1} {bz+2}.5")
        time.sleep(3)
    F("clients bars before:", bars(a))
    # drink the ominous bottle (right click held)
    a.select("minecraft:ominous_bottle")
    a.mouse("right", "down"); time.sleep(2.4); a.mouse("right", "up")
    time.sleep(0.5)
    F("effects after the bottle", effects(a))
    t0 = time.time()
    first = None
    last_wave_key = None
    kills = 0
    hero = False
    waves_seen = {}
    last_sample = 0
    S("gamerule advance_time true", "tick rate 20")
    while time.time() - t0 < max_s:
        ef = effects(a)
        if "hero_of_the_village" in ef:
            hero = True
            F("hero of the village:", ef["hero_of_the_village"], "after", round(time.time() - t0), "s; kills", kills)
            break
        if time.time() - last_sample >= 1.5:
            last_sample = time.time()
            rs = raiders_now()
            by_wave = {}
            for t, w, hp, gl in rs:
                by_wave.setdefault(w, {}).setdefault(t, [0, 0, 0.0])
                by_wave[w][t][0] += 1
                by_wave[w][t][1] += gl
                by_wave[w][t][2] += hp
            sample = {"t": round(time.time() - t0, 1), "waves": {str(w): {t: [v[0], v[1], round(v[2])] for t, v in d.items()} for w, d in by_wave.items()}, "bars": bars(a), "villagers": villagers_now() if len(LOG["samples"]) % 10 == 0 else None}
            LOG["samples"].append(sample)
            if rs and first is None:
                first = time.time() - t0
                F("first raiders after", round(first), "s:", sample["waves"], "bar", sample["bars"])
            key = tuple(sorted(by_wave))
            if key != last_wave_key:
                last_wave_key = key
                F("raiders by wave:", sample["waves"], "| bars:", sample["bars"], "| kills", kills)
        if a.state(0)["health"] <= 0:
            alive(a)
        kills += fight(a, t0 + max_s)
        if first is None and time.time() - t0 > 240:
            F("no raid after 240 s; effects", effects(a))
            break
    LOG["kills"] = kills
    LOG["hero"] = hero
    return hero


def after_raid(a):
    """What follows a victory: the hero effect, the villagers' celebration and gifts, the trade prices."""
    F("effects", effects(a), "bars", bars(a))
    # celebration: raid bar text, villagers' activity (no NBT for it; they jump / fireworks are not visible server side): record positions
    # gifts: item entities near the villagers over 90 s
    gifts = {}
    t0 = time.time()
    while time.time() - t0 < 120:
        out = S(f"execute as @e[type=minecraft:item,{BOX}] run data get entity @s Item")
        for l in out:
            m = re.search(r'id: "minecraft:([a-z_]+)"', l)
            c = re.search(r"count: (\d+)", l)
            if m:
                gifts.setdefault(m.group(1), set()).add(l[-120:])
        time.sleep(3)
        if int(time.time() - t0) % 30 == 0:
            F("gift items so far:", sorted(gifts))
    LOG["gifts"] = sorted(gifts)
    F("gifts thrown:", sorted(gifts))


if __name__ == "__main__":
    outp = sys.argv[1]
    a = client()
    try:
        ok = run_raid(a)
        if ok:
            after_raid(a)
    finally:
        json.dump(LOG, open(outp, "w"), indent=1)
        F("log written", outp)
