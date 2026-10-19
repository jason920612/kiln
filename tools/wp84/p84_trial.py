"""wp84 #3: an ominous trial the way a player gets one: drink an ominous bottle (Bad Omen), walk into a trial chamber's spawner room, Bad Omen turns into Trial Omen, the
spawner turns ominous, items rain from the ominous item spawners, mobs spawn; then the ominous vault with the ominous trial key. One actor against each server (par.sh).
usage: par.sh p84_trial.py [omen,vault]"""
from p72 import *
import collections

a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
PH = (sys.argv[1] if len(sys.argv) > 1 else "omen,vault").split(",")
scenario("trial84")
CX, CY, CZ = 230, -30, 230


def lines():
    return json.loads(a.cmd("spylog clear"))


def gametime():
    out = sv("time query gametime", show=False, wait=0.4)
    m = re.findall(r"(\d+)\]?", out)
    return int(m[-1]) if m else None


def effects():
    out = sv(f"data get entity {ME} active_effects", show=False, wait=0.5)
    return sorted(set(re.findall(r'id: "minecraft:(\w+)"', out))), re.findall(r"amplifier: (\d+)b", out), re.findall(r"duration: (\d+)", out)


def find_block(name, n=10):
    r = a.cmd(f"findbox {name} {CX - 60} -60 {CZ - 60} {CX + 60} 0 {CZ + 60} {n}")
    try:
        return json.loads(r)
    except Exception:
        return []


def setup():
    sv(f"gamemode creative {ME}", show=False)
    sv(f"tp {ME} {CX}.5 20 {CZ}.5", show=False)
    sv(f"forceload add {CX - 50} {CZ - 50} {CX + 50} {CZ + 50}", show=False)
    time.sleep(12)
    for _ in range(12):
        out = sv(f"place structure minecraft:trial_chambers {CX} {CY} {CZ}", show=False, wait=8.0)
        if "unloaded" not in out:
            break
        time.sleep(10)
    F("place:", out[-100:])
    time.sleep(5)
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("difficulty normal", show=False)
    sv(f"gamemode survival {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"effect give {ME} minecraft:regeneration 99999 2 true", show=False)
    for slot, it in (("armor.head", "iron_helmet"), ("armor.chest", "iron_chestplate"), ("armor.legs", "iron_leggings"), ("armor.feet", "iron_boots")):
        sv(f"item replace entity {ME} {slot} with minecraft:{it}", show=False)
    for it in ("minecraft:diamond_sword", "minecraft:golden_apple 16", "minecraft:shield"):
        give(ME, it)


def spawner_state(pos):
    b = blk(a, *pos)
    m = re.search(r"trial_spawner_state=(\w+)", b)
    om = re.search(r"ominous=(\w+)", b)
    return (m.group(1) if m else b[:30], om.group(1) if om else None)


def omen():
    sp = find_block("trial_spawner", 12)
    F("trial spawners seen by the client (count):", len(sp), "first", sp[:3])
    if not sp:
        return
    # the room nearest to the origin of the structure
    sp.sort(key=lambda p: (p[0] - CX) ** 2 + (p[2] - CZ) ** 2)
    S = tuple(sp[0])
    F("chosen spawner:", S)
    sv(f"tp {ME} {S[0] + 0.5} {S[1] + 8} {S[2] + 0.5}", show=False)
    time.sleep(2)
    # no natural spawns beside the spawner: light the room (the spawner needs `spawn_mobs`)
    for dx in range(-9, 10, 3):
        for dz in range(-9, 10, 3):
            for dy in (1, 2):
                sv(f"execute if block {S[0] + dx} {S[1] + dy} {S[2] + dz} minecraft:air run setblock {S[0] + dx} {S[1] + dy} {S[2] + dz} minecraft:light[level=15]", show=False, wait=0.05)
    # the ominous bottle: a real drink
    sv(f"give {ME} minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier=1] 1", show=False)
    a.select("minecraft:ominous_bottle")
    rec_start(a)
    a.mouse("right", "down")
    time.sleep(2.2)
    a.mouse("right", "up")
    time.sleep(0.8)
    cnt = rec_stop(a, "bottle", show=False)
    F("after the ominous bottle: effects", effects(), "bottle left", a.inv().get("minecraft:ominous_bottle", 0), "fx", sorted(k for k in cnt if k.startswith(("sound", "UpdateMobEffect", "entity_event")))[:6])
    # walk into the room: put the player beside the spawner and poll
    spot = (S[0] + 0.5, S[1] + 1, S[2] + 2.5)
    for dx, dz in ((0, 2), (0, -2), (2, 0), (-2, 0), (2, 2), (-2, -2)):
        if blk(a, S[0] + dx, S[1] + 1, S[2] + dz).startswith(("air", "light")) and blk(a, S[0] + dx, S[1] + 2, S[2] + dz).startswith(("air", "light")):
            spot = (S[0] + dx + 0.5, S[1] + 1, S[2] + dz + 0.5)
            break
    sv(f"tp {ME} {spot[0]} {spot[1]} {spot[2]}", show=False)
    F("standing at", spot)
    t0 = time.time()
    g0 = gametime()
    seq, last = [], None
    items = collections.Counter()
    ents = collections.Counter()
    rec_start(a)
    while time.time() - t0 < 150:
        alive_auto(a)
        eff = effects()
        st = spawner_state(S)
        key = (tuple(eff[0]), eff[1], st)
        if key != last:
            seq.append((gametime() - g0, eff[0], eff[1], st))
            last = key
        for e in a.state(30)["entities"]:
            ents[e["t"]] += 0
        near = [e for e in a.state(12)["entities"] if e["t"] in ("slime", "zombie", "skeleton", "spider", "breeze", "bogged", "cave_spider", "silverfish", "baby_zombie")]
        if near:
            kill_nearest(a, near[0]["t"], rng=12, timeout=5)
        sv("kill @e[type=slime,distance=..40]", show=False, wait=0.2)
        for e in a.state(30)["entities"]:
            if e["t"] in ("ominous_item_spawner", "item", "wind_charge", "arrow", "spectral_arrow", "small_fireball", "potion", "breeze", "zombie", "skeleton", "slime", "bogged", "cave_spider", "spider", "silverfish", "baby_zombie"):
                ents[e["t"]] = max(ents[e["t"]], 1) if ents[e["t"]] == 0 else ents[e["t"]]
        if st[0] == "cooldown":
            break
        time.sleep(0.5)
    ls = lines()
    cnt = collections.Counter(re.sub(r"-?\d+\.\d+", "#", l)[:60] for l in ls if l.split(" ")[0] in ("AddEntity", "add_entity") or "ominous" in l)
    F("sequence (ticks since the drink, effects, amplifiers, spawner state/ominous):", seq)
    F("entity kinds seen:", sorted(k for k, v in ents.items() if v))
    F("ominous-related packets:", dict(cnt))
    F("spawner data:", sv(f"data get block {S[0]} {S[1]} {S[2]}", show=False, wait=0.8)[-700:])


def vault():
    vs = find_block("vault", 10)
    F("vaults seen:", vs[:5])
    sv(f"give {ME} minecraft:ominous_trial_key 4", show=False)
    sv(f"give {ME} minecraft:trial_key 4", show=False)
    for v in vs[:4]:
        b = blk(a, *v)
        F("vault", v, b)
        sv(f"tp {ME} {v[0] + 0.5} {v[1]} {v[2] + 2.5}", show=False)
        time.sleep(1.5)
        key = "minecraft:ominous_trial_key" if "ominous=true" in b else "minecraft:trial_key"
        a.select(key)
        rec_start(a)
        ok = a.use_on(v[0], v[1], v[2])
        time.sleep(3.0)
        cnt = rec_stop(a, "vault", show=False)
        a.collect(6, 3)
        F("vault use", v, "ok", ok, "state", blk(a, *v)[:60], "keys left", a.inv().get(key, 0), "items", {k: n for k, n in a.inv().items() if "key" not in k and "golden_apple" not in k and "diamond_sword" not in k and "shield" not in k and "iron_" not in k}, "fx", sorted(k for k in cnt if k.startswith(("sound", "particle")))[:8])


FN = {"omen": omen, "vault": vault}
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
setup()
for ph in PH:
    try:
        FN[ph]()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR in", ph, repr(ex)[:300])
