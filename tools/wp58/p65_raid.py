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
        sv(f"effect give {nm} minecraft:resistance 3000 4 true", show=False)
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
    t0 = time.time(); log = []; last = None; started = None; since_wave = None
    while time.time() - t0 < 900:
        alive(a); alive(b)
        time.sleep(3)
        c = counts()
        tot = sum(c.values())
        if tot and started is None:
            started = time.time() - t0
            F("first raiders after", round(started), "s", c)
            shot("raid_wave1")
        if tot and c != last:
            log.append((round(time.time() - t0), dict(c)))
            last = c
            since_wave = time.time()
        for cl in (a, b):
            cl.select("minecraft:diamond_sword")
            es = [e for e in cl.state(40)["entities"] if e["t"] in RAIDERS]
            if es and cl.state(0)["health"] > 8:
                kill_nearest(cl, es[0]["t"], rng=40, timeout=6, dy=0.9)
        # real fighting for 25 s of a wave, then the rest is cleaned so the next wave comes
        if tot and since_wave and time.time() - since_wave > 25:
            sv("kill @e[type=#minecraft:raiders,x=1300,y=0,z=-1200,dx=300,dy=320,dz=300]", show=False)
            since_wave = time.time()
        eff = [e["id"].split(":")[-1] for e in a.state(0)["effects"]]
        if "hero_of_the_village" in eff:
            F("hero of the village after", round(time.time() - t0), "s")
            break
        if started is None and time.time() - t0 > 200:
            F("no raid started; effects", eff)
            break
    sv("tick rate 20", show=False)
    F("wave log", log)
    F("effects A", [e["id"].split(":")[-1] + str(e["amp"]) for e in a.state(0)["effects"]], "B", [e["id"].split(":")[-1] + str(e["amp"]) for e in b.state(0)["effects"]])
    adv = chat()
    F("chat", [l for l in adv if "advancement" in l or "aid" in l or "Hero" in l][:12])
    shot("raid_end")


if __name__ == "__main__":
    raid()
