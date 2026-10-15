"""wp65: a full village raid with two real players: both drink an ominous bottle (Bad Omen III) in a real village, the raid is
followed wave by wave (counts per raider type, boss bar), the raiders are fought (real melee for a while, then cleaned) until Hero of the Village.
Kiln (TARGET=K) vs vanilla (TARGET=V); the village is the one `locate` finds (same place on both)."""
from p65_b2 import *

RAIDERS = ["pillager", "vindicator", "ravager", "evoker", "witch", "vex", "illusioner"]


def counts(box="x=1300,y=0,z=-1200,dx=300,dy=320,dz=300"):
    out = {}
    r = sv(f"execute as @e[type=#minecraft:raiders,{box}] run data get entity @s Health", show=False, wait=0.9)
    for line in r.splitlines():
        for t in RAIDERS:
            if re.search(r"(?<![a-z_])%s(?![a-z_])" % t, line.lower()):
                out[t] = out.get(t, 0) + 1
    return out


def raid():
    prep()
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"gamemode survival {nm}", show=False)
        sv(f"give {nm} minecraft:diamond_sword 1", show=False)
        sv(f"give {nm} minecraft:diamond_chestplate 1", show=False)
        sv(f"give {nm} minecraft:golden_apple 16", show=False)
        sv(f"give {nm} minecraft:ominous_bottle[minecraft:ominous_bottle_amplifier=2] 1", show=False)
        sv(f"effect give {nm} minecraft:resistance 3000 3 true", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("difficulty normal", show=False)
    sv("gamerule raids true", show=False)
    sv("time set day", show=False)
    sv("gamerule advance_time false", show=False)
    sv("forceload add 1440 -1056 1472 -1024", show=False)
    time.sleep(3)
    a_ground = a.safe_tp(1456, -1040, 90)
    sv("tp Obs KilnTest", show=False)
    time.sleep(6)
    F("village: A at", pos(a), "villagers around", len([e for e in a.state(60)["entities"] if e["t"] == "villager"]), "iron golems", len([e for e in a.state(60)["entities"] if e["t"] == "iron_golem"]))
    bells = a.find("bell", 100, 5)
    F("bells", bells, "beds", len(a.find("_bed", 100, 40)))
    if bells:
        bx, by, bz = bells[0]
        sv(f"tp KilnTest {bx + 0.5} {by + 1.0} {bz + 2.5}", show=False)
        sv("tp Obs KilnTest", show=False)
        time.sleep(4)
        F("moved to the bell", pos(a))
    shot("raid_village")
    # real: both drink the ominous bottle
    for c in (a, b):
        c.select("minecraft:ominous_bottle")
        c.mouse("right", "down"); time.sleep(2.2); c.mouse("right", "up"); time.sleep(0.5)
    F("effects after drinking: A", [e["id"].split(":")[-1] + str(e["amp"]) for e in a.state(0)["effects"]], "B", [e["id"].split(":")[-1] + str(e["amp"]) for e in b.state(0)["effects"]])
    sv("tick rate 60", show=False)
    t0 = time.time(); wave = 0; seen = {}
    log = []
    last = None
    started = None
    while time.time() - t0 < 600 and not (started is None and time.time() - t0 > 100):
        alive(a); alive(b)
        time.sleep(4)
        c = counts()
        tot = sum(c.values())
        if tot and started is None:
            started = time.time() - t0
            F("first raiders after", round(started), "s", c)
            shot("raid_wave1")
        if c != last and tot:
            log.append((round(time.time() - t0), c))
            last = c
        # fight a bit for real: the nearest raider
        for cl in (a, b):
            cl.select("minecraft:diamond_sword")
            es = [e for e in cl.state(40)["entities"] if e["t"] in RAIDERS]
            if es and cl.state(0)["health"] > 8:
                kill_nearest(cl, es[0]["t"], rng=40, timeout=8, dy=0.9)
            if cl.state(0)["health"] < 9:
                sv("effect give %s minecraft:instant_health 1 3 true" % ("KilnTest" if cl is a else "Obs"), show=False)
        # when the same group of raiders has stayed 60 s, clean them (the real fighting is slow)
        if started is not None and tot == 0 and time.time() - t0 - started > 20:
            r = sv("execute if entity @e[type=minecraft:pillager,x=1300,y=0,z=-1200,dx=300,dy=320,dz=300]", show=False)
            eff = [e["id"].split(":")[-1] for e in a.state(0)["effects"]]
            F("raiders gone; A effects", eff)
            if "hero_of_the_village" in eff:
                break
        if int(time.time() - t0) % 60 < 5:
            F("t+%d" % (time.time() - t0), c, "bossbar", sv("bossbar list", show=False)[-90:])
        # clean raiders that stay too long so the raid can end
        if time.time() - t0 > 150 and int(time.time() - t0) % 45 < 5 and tot:
            sv("kill @e[type=#minecraft:raiders,x=1300,y=0,z=-1200,dx=300,dy=320,dz=300]", show=False)
    sv("tick rate 20", show=False)
    F("wave log", log)
    F("hero", [e["id"].split(":")[-1] + str(e["amp"]) for e in a.state(0)["effects"]], [e["id"].split(":")[-1] for e in b.state(0)["effects"]])
    adv = chat()
    F("chat", [l for l in adv if "advancement" in l or "Raid" in l or "raid" in l][:12])
    shot("raid_end")


if __name__ == "__main__":
    raid()
