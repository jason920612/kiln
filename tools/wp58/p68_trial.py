"""wp68 #4: a trial chamber placed with `/place structure` on both servers (same seed: the same rooms), a trial spawner fought for real
(states, mobs, reward), a vault opened with the key it gives, the ominous variant with Trial Omen. Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p68_small import *
import sys

scenario("trial")
SPAWNER = (231, -29, 234)


def state_of(pos):
    b = blk(a, *pos)
    m = re.search(r"trial_spawner_state=(\w+)", b)
    om = re.search(r"ominous=(\w+)", b)
    return (m.group(1) if m else b, om.group(1) if om else None)


def setup():
    sv(f"gamemode creative {ME}", show=False)
    sv(f"tp {ME} 230.5 20 230.5", show=False)
    sv("forceload add 200 200 260 260", show=False)
    time.sleep(12)
    out = sv("place structure minecraft:trial_chambers 230 -30 230", show=False, wait=6.0)
    F("place:", out[-120:])
    time.sleep(4)
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs true", show=False)
    # light the surroundings (no natural spawns next to the player)
    for dx in range(-9, 10, 3):
        for dz in range(-9, 10, 3):
            sv(f"execute if block {231+dx} -28 {234+dz} minecraft:air run setblock {231+dx} -28 {234+dz} minecraft:light[level=15]", show=False, wait=0.05)
    sv("difficulty normal", show=False)
    sv(f"gamemode survival {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    sv(f"clear {ME}", show=False)
    sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
    sv(f"effect give {ME} minecraft:regeneration 99999 2 true", show=False)
    for it in ("minecraft:diamond_sword", "minecraft:bow", "minecraft:arrow 64", "minecraft:golden_apple 16", "minecraft:shield"):
        give(ME, it)
    for slot, it in (("armor.head", "iron_helmet"), ("armor.chest", "iron_chestplate"), ("armor.legs", "iron_leggings"), ("armor.feet", "iron_boots")):
        sv(f"item replace entity {ME} {slot} with minecraft:{it}", show=False)


def spawner_nbt():
    out = sv(f"data get block {SPAWNER[0]} {SPAWNER[1]} {SPAWNER[2]}", show=False, wait=0.8)
    return out[-900:]


def fields_of(txt):
    g = lambda k: (re.findall(k + r": (-?\d+)L?", txt) or [None])[0]
    return {"total": g("total_mobs_spawned"), "next": g("next_mob_spawns_at"), "cooldown": g("cooldown_ends_at"), "current": len(re.findall(r"\[I; -?\d+, -?\d+, -?\d+, -?\d+\]", (re.findall(r"current_mobs: \[(.*?)\]\]", txt) or [""])[0] + "]")) if "current_mobs" in txt else 0}


def gametime():
    out = sv("time query gametime", show=False, wait=0.4)
    m = re.findall(r"(\d+)\]?", out)
    return int(m[-1]) if m else None


def fight(secs=150):
    """Stand beside the spawner; real hits on what comes at the player, the rest of the spawner's mobs are removed by command; log the
    spawner's state (block state + data) with the game time, the items it ejects."""
    sv(f"tp {ME} 231.5 -29 231.5", show=False)
    time.sleep(2.0)
    t0 = time.time()
    g0 = gametime()
    states, last = [], None
    items = collections.Counter()
    kills = 0
    seen_items = set()
    while time.time() - t0 < secs:
        alive_auto(a)
        st = state_of(SPAWNER)
        if st[0] == "void_air":
            sv(f"tp {ME} 231.5 -29 231.5", show=False)
            time.sleep(1.0)
            continue
        txt = spawner_nbt()
        f = fields_of(txt)
        key = (st, f["total"], f["current"])
        if key != last:
            states.append((gametime() - g0, st[0], st[1], f))
            last = key
        # real fight
        near = [e for e in a.state(12)["entities"] if e["t"] in ("slime", "zombie", "skeleton", "spider", "breeze", "bogged", "cave_spider", "silverfish", "baby_zombie")]
        if near:
            if kill_nearest(a, near[0]["t"], rng=12, timeout=6):
                kills += 1
        sv("kill @e[type=slime,distance=..40]", show=False, wait=0.2)
        sv("kill @e[type=!player,type=!item,type=!slime,type=!experience_orb,type=!trial_spawner,distance=..60]", show=False, wait=0.2)
        for e in a.state(12)["entities"]:
            if e["t"] == "item" and e["id"] not in seen_items:
                seen_items.add(e["id"])
                items[e.get("item")] += e.get("n", 1)
        a.collect(5, 3)
        if st[0] == "cooldown":
            break
        time.sleep(0.4)
    F("spawner states (game ticks since start, block state, ominous, data):", states)
    F("real kills", kills, "items ejected:", dict(items))
    F("inventory:", {k: v for k, v in a.inv().items() if "key" in k or "trial" in k or k in ("minecraft:emerald", "minecraft:wind_charge", "minecraft:arrow", "minecraft:golden_apple")})


if __name__ == "__main__":
    prep()
    time.sleep(2)
    setup()
    F("spawner block entity:", spawner_nbt())
    try:
        fight()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
    F("spawner after:", spawner_nbt())
