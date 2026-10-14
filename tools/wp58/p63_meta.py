"""wp63 gap 1/2: what the server sends about a player (entity data to the observer and to the player itself) and what it
sends again on respawn, dimension change, gamemode change, /op and /deop. Runs on the VM next to the server and the two
real clients; TARGET=V runs it against the vanilla reference server.

  KILN_PLAY_DIR=wp63 PYTHONPATH=tools/wp58 python3 tools/wp58/p63_meta.py meta     # entity data per event
  KILN_PLAY_DIR=wp63 PYTHONPATH=tools/wp58 python3 tools/wp58/p63_meta.py session  # packets of respawn / dimension / gamemode / op

Output: ~/wp63/<what>_<K|V>.json (event -> {"obs": [...], "self": [...]}).
"""
import json, os, re, sys, time
from pathlib import Path
import p61
from p61 import *

p61.VDIR = HOME / "wp63" / "vanilla"
T = os.environ.get("TARGET", "K")
OUT = HOME / "wp63"


def spylog(c, clear=True):
    return json.loads(c.cmd("spylog clear" if clear else "spylog"))


NOISE = ("Sound", "sound", "LevelParticles", "PlayerInfo", "SetPlayerTeam", "UpdateAdvancements", "Pos", "PosRot", "TrackedWaypoint", "SystemChat", "RotateHead", "MoveEntity")


def keep(lines, drop=NOISE):
    out = []
    for l in lines:
        if l in drop or any(l.startswith(d + " ") for d in drop):
            continue
        out.append(re.sub(r"@[0-9a-f]+", "", l))
    return out


class Both:
    """The actor's client (25999) and the observer's (26000), both spying."""

    def __init__(self):
        self.a = C(25999)
        self.o = C(26000)
        self.a.cmd("spy on")
        self.o.cmd("spy on")

    def event(self, name, action, wait=1.5, filt=True):
        if alive(self.a) <= 0:
            raise SystemExit("the actor is dead before " + name)
        spylog(self.a)
        spylog(self.o)
        action()
        time.sleep(wait)
        o, s = spylog(self.o), spylog(self.a)
        if filt:
            o, s = keep(o), keep(s)
        print(f"== {name}\n  obs : {o}\n  self: {s}", flush=True)
        return {"obs": o, "self": s}


def setup(b):
    """Both players on the platform, actor in survival with full health."""
    if T == "V":
        sv("op KilnTest", show=False)
    platform(-20, -20, 20, 20, y=199, clear=12)
    sv("gamemode survival KilnTest", show=False)
    sv("gamemode creative Obs", show=False)
    sv("effect clear KilnTest", show=False)
    sv("attribute KilnTest minecraft:max_health base set 20", show=False)
    sv("tp KilnTest 0 200 0", show=False)
    sv("tp Obs 3 200 3", show=False)
    time.sleep(2.5)
    sv("effect give KilnTest minecraft:instant_health 1 5", show=False)
    time.sleep(1.5)
    alive(b.a)


def meta(b):
    ev = {}
    sv("gamemode creative KilnTest", show=False)
    time.sleep(1)
    ev["tp_away_back"] = b.event("tp away and back (re-pairing: spawn data)", lambda: (sv("tp KilnTest 3000 200 3000", show=False), time.sleep(3), sv("tp KilnTest 0 200 0", show=False)), wait=4)
    sv("gamemode survival KilnTest", show=False)
    time.sleep(1.5)
    ev["damage4"] = b.event("damage 4", lambda: sv("damage KilnTest 4 minecraft:generic", show=False))
    time.sleep(1)
    ev["heal"] = b.event("instant_health", lambda: sv("effect give KilnTest minecraft:instant_health 1 1", show=False))
    ev["damage_tiny"] = b.event("damage 0.5", lambda: sv("damage KilnTest 0.5 minecraft:generic", show=False))
    ev["absorption"] = b.event("absorption II 30s", lambda: sv("effect give KilnTest minecraft:absorption 30 1", show=False))
    ev["damage_absorbed"] = b.event("damage 2 (absorbed)", lambda: sv("damage KilnTest 2 minecraft:generic", show=False))
    ev["effects_clear"] = b.event("effect clear", lambda: sv("effect clear KilnTest", show=False))
    ev["invisible"] = b.event("invisibility", lambda: sv("effect give KilnTest minecraft:invisibility 30 0 true", show=False))
    ev["glowing"] = b.event("glowing", lambda: sv("effect give KilnTest minecraft:glowing 30 0 true", show=False))
    ev["speed_particles"] = b.event("speed (particles)", lambda: sv("effect give KilnTest minecraft:speed 30 0", show=False))
    ev["effects_clear2"] = b.event("effect clear 2", lambda: sv("effect clear KilnTest", show=False))
    sv("attribute KilnTest minecraft:max_health base set 30", show=False)
    ev["maxhealth30"] = b.event("max_health 30 + heal", lambda: (sv("effect give KilnTest minecraft:instant_health 1 5", show=False)))
    sv("attribute KilnTest minecraft:max_health base set 20", show=False)
    ev["maxhealth20"] = b.event("max_health back to 20", lambda: time.sleep(0.2))
    ev["sneak"] = b.event("sneak down", lambda: b.a.key("shift", "down"))
    ev["sneak_up"] = b.event("sneak up", lambda: b.a.key("shift", "up"))
    ev["sprint"] = b.event("sprint forward 1 s", lambda: (b.a.key("w", "down"), b.a.key("ctrl", "down"), time.sleep(1.0), b.a.key("ctrl", "up"), b.a.key("w", "up")), wait=1.0)
    sv("tp KilnTest 0 200 0", show=False)
    time.sleep(1.5)
    ev["fire"] = b.event("on fire (fire block)", lambda: (sv("setblock 0 200 0 minecraft:fire", show=False)), wait=2.0)
    sv("setblock 0 200 0 minecraft:air", show=False)
    sv("tp KilnTest 0 200 0", show=False)
    time.sleep(3)
    ev["after_fire"] = b.event("after fire (extinguish wait)", lambda: time.sleep(4), wait=1.0)
    fill(-3, 200, -3, 3, 204, 3, "minecraft:water")
    ev["water_air"] = b.event("submerged: air supply", lambda: sv("tp KilnTest 0 202 0", show=False), wait=3.0)
    sv("fill -3 200 -3 3 204 3 minecraft:air", show=False)
    time.sleep(1.5)
    ev["air_back"] = b.event("out of water: air refill", lambda: sv("tp KilnTest 0 200 0", show=False), wait=2.0)
    ev["freeze"] = b.event("powder snow: ticks frozen", lambda: sv("setblock 0 200 0 minecraft:powder_snow", show=False) or sv("tp KilnTest 0 200.5 0", show=False), wait=3.0)
    sv("setblock 0 200 0 minecraft:air", show=False)
    time.sleep(1)
    ev["arrow"] = b.event("an arrow hits", lambda: sv("summon minecraft:arrow 0.5 203 0.5 {Motion:[0.0,-1.5,0.0],damage:0.5}", show=False), wait=2.0)
    ev["xp_score"] = b.event("xp levels (score unaffected)", lambda: sv("xp add KilnTest 7 levels", show=False))
    ev["quiet"] = b.event("quiet second", lambda: time.sleep(0.1), wait=1.2)
    return ev


def session(b):
    ev = {}
    ev["gm_creative"] = b.event("/gamemode creative", lambda: sv("gamemode creative KilnTest", show=False), filt=False)
    ev["gm_survival"] = b.event("/gamemode survival", lambda: sv("gamemode survival KilnTest", show=False), filt=False)
    ev["gm_spectator"] = b.event("/gamemode spectator", lambda: sv("gamemode spectator KilnTest", show=False), filt=False)
    ev["gm_survival2"] = b.event("/gamemode survival", lambda: sv("gamemode survival KilnTest", show=False), filt=False)
    ev["gm_same"] = b.event("/gamemode survival (same)", lambda: sv("gamemode survival KilnTest", show=False), filt=False)
    ev["deop"] = b.event("/deop", lambda: sv("deop KilnTest", show=False), filt=False)
    ev["op"] = b.event("/op", lambda: sv("op KilnTest", show=False), filt=False)
    ev["op_again"] = b.event("/op (already op)", lambda: sv("op KilnTest", show=False), filt=False)
    ev["nether"] = b.event("to the nether", lambda: sv("execute in minecraft:the_nether run tp KilnTest 0 100 0", show=False), wait=4, filt=False)
    ev["overworld"] = b.event("back to the overworld", lambda: sv("execute in minecraft:overworld run tp KilnTest 0 200 0", show=False), wait=4, filt=False)
    ev["same_dim_tp"] = b.event("same-dimension tp", lambda: sv("tp KilnTest 5 200 5", show=False), wait=2, filt=False)
    sv("effect give KilnTest minecraft:speed 100 0", show=False)
    ev["kill"] = b.event("/kill", lambda: sv("kill KilnTest", show=False), wait=2, filt=False)

    def respawn():
        c = b.a
        for _ in range(4):
            if c.screen() != "DeathScreen":
                break
            c.cmd("mousepos 480 298"); time.sleep(1.5)
            c.mouse("left", "tap"); time.sleep(2.5)
    ev["respawn"] = b.event("respawn click", respawn, wait=3, filt=False)
    return ev


if __name__ == "__main__":
    what = sys.argv[1]
    b = Both()
    setup(b)
    res = meta(b) if what == "meta" else session(b)
    p = OUT / f"{what}_{T}.json"
    p.write_text(json.dumps(res, indent=1))
    print("wrote", p)
