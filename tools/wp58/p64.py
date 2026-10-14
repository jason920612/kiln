"""wp64: what the server sends about the five small player-sync differences wp63 left, side by side with vanilla.
Runs on the VM next to the server and the two real clients (actor KilnTest on 25999, observer Obs on 26000);
TARGET=V runs it against the vanilla reference server (wp64_vanilla.sh).

  KILN_PLAY_DIR=wp64 PYTHONPATH=tools/wp58 python3 tools/wp58/p64.py arrows|sting|spectator|powder|sneak|instant|all

Every event prints the packets of the observer and of the actor in arrival order, grouped by server tick (packets
less than 20 ms apart), with the time since the event: "[ +123ms] line". Output: ~/wp64/<scenario>_<K|V>.json.
"""
import json, os, re, sys, time
from pathlib import Path
import p61
from p61 import *

p61.VDIR = HOME / "wp64" / "vanilla"
T = os.environ.get("TARGET", "K")
OUT = HOME / "wp64"

DROP = ("KeepAlive", "Pos", "PosRot", "Rot", "RotateHead", "MoveEntity", "TrackedWaypoint", "SystemChat", "SetTime", "UpdateAdvancements",
        "SetChunkCacheCenter", "ForgetLevelChunk", "LevelChunkWithLight", "LightUpdate", "ChunkBatchStart", "ChunkBatchFinished")


def spylog_ts(c):
    return json.loads(c.cmd("spylog ts clear"))


def tidy(lines, drop=DROP, sounds=False, drop_re=None):
    """[(ms, text)] -> grouped by tick: ["[+t ms] text", ...] with the position hashes removed."""
    out = []
    last = None
    for l in lines:
        ms, _, text = l.partition(" ")
        ms = int(ms)
        text = re.sub(r"@[0-9a-f]+", "", text)
        name = text.split(" ")[0]
        if name in drop:
            continue
        if not sounds and (name in ("sound", "sound_entity") or text.startswith("Sound")):
            continue
        if text.startswith("player_info [UPDATE_LATENCY]"):
            continue
        if drop_re and re.search(drop_re, text):
            continue
        out.append((ms, text))
    res = []
    t0 = out[0][0] if out else 0
    for ms, text in out:
        mark = "      " if last is not None and ms - last < 20 else f"+{ms - t0:5d}"
        # (the first line of a group carries its time since the first line of the event)
        res.append(f"[{mark}] {text}")
        last = ms
    return res


class Both:
    """The actor's client (25999) and the observer's (26000), both spying."""

    def __init__(self):
        self.a = C(25999)
        self.o = C(26000)
        self.a.cmd("spy on")
        self.o.cmd("spy on")

    def event(self, name, action, wait=1.5, sounds=False, need_alive=True, drop_re=None):
        if need_alive and alive(self.a) <= 0:
            raise SystemExit("the actor is dead before " + name)
        spylog_ts(self.a)
        spylog_ts(self.o)
        action()
        time.sleep(wait)
        o, s = spylog_ts(self.o), spylog_ts(self.a)
        o, s = tidy(o, sounds=sounds, drop_re=drop_re), tidy(s, sounds=sounds, drop_re=drop_re)
        print(f"== {name}\n  obs :", *o, sep="\n    ")
        print("  self:", *s, sep="\n    ", flush=True)
        return {"obs": o, "self": s}


def setup(b, mode="survival"):
    """Both players on the platform, actor in survival with full health."""
    if T == "V":
        sv("op KilnTest", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("difficulty normal", show=False)
    sv("kill @e[type=!player]", show=False)
    platform(-20, -20, 20, 20, y=199, clear=12)
    sv("gamemode creative Obs", show=False)
    sv(f"gamemode {mode} KilnTest", show=False)
    sv("effect clear KilnTest", show=False)
    sv("attribute KilnTest minecraft:max_health base set 20", show=False)
    sv("tp KilnTest 0 200 0", show=False)
    sv("tp Obs 3 200 3", show=False)
    time.sleep(2.5)
    sv("effect give KilnTest minecraft:instant_health 1 5", show=False)
    time.sleep(1.5)
    alive(b.a)


def uuid_of(name):
    out = sv(f"data get entity {name} UUID", wait=0.8, show=False)
    m = re.search(r"\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", out)
    return m.groups() if m else None


def arrows(b):
    ev = {}
    sv("tick rate 100", show=False)
    time.sleep(0.5)
    ev["arrow1"] = b.event("one arrow hits", lambda: sv("summon minecraft:arrow 0.2 203 0.2 {Motion:[0.0,-1.5,0.0]}", show=False), wait=2.0)
    ev["arrow_timer"] = b.event("(the arrow count drops after 20*(30-1) = 580 ticks)", lambda: time.sleep(5), wait=2.0)
    ev["arrow3"] = b.event("three arrows, one tick apart", lambda: [(sv("summon minecraft:arrow 0.2 203 0.2 {Motion:[0.0,-1.5,0.0]}", show=False, wait=0.03)) for _ in range(3)], wait=2.0)
    ev["arrow3_timer"] = b.event("(580 ticks for 3->2, then 20*(30-2) = 560 for 2->1, then 600 for 1->0)", lambda: time.sleep(16), wait=2.0)
    ev["arrow_creative"] = b.event("an arrow hits a creative player", lambda: (sv("gamemode creative KilnTest", show=False), time.sleep(1.0), sv("summon minecraft:arrow 0.2 203 0.2 {Motion:[0.0,-1.5,0.0]}", show=False)), wait=2.0)
    sv("gamemode survival KilnTest", show=False)
    sv("tick rate 20", show=False)
    return ev


def sting(b):
    ev = {}
    u = uuid_of("KilnTest")
    print("uuid", u)
    if not u:
        raise SystemExit("no uuid")
    sv("tick rate 100", show=False)
    gt = sv("time query gametime", show=False)
    nbt = "{angry_at:[I;%s,%s,%s,%s],anger_end_time:9999999L}" % u
    ev["sting"] = b.event("an angry bee stings", lambda: sv(f"summon minecraft:bee 0.5 201 0.5 {nbt}", show=False), wait=3.0)
    ev["sting_timer"] = b.event("(the stinger count drops after 580 ticks)", lambda: time.sleep(8), wait=2.0)
    sv("kill @e[type=minecraft:bee]", show=False)
    sv("tick rate 20", show=False)
    return ev


def spectator(b):
    ev = {}
    # the observer in creative, the actor survival -> spectator -> survival
    ev["obs_creative__a_spectator"] = b.event("obs creative: actor -> spectator", lambda: sv("gamemode spectator KilnTest", show=False), wait=3.0, sounds=False)
    ev["obs_creative__a_survival"] = b.event("obs creative: actor -> survival", lambda: sv("gamemode survival KilnTest", show=False), wait=3.0)
    sv("gamemode spectator KilnTest", show=False)
    time.sleep(3)
    # the observer becomes a spectator while the actor already is one
    ev["obs_spectator__a_spectator"] = b.event("actor spectator: obs -> spectator", lambda: sv("gamemode spectator Obs", show=False), wait=3.0)
    ev["both_spectators_actor_survival"] = b.event("both spectators: actor -> survival", lambda: sv("gamemode survival KilnTest", show=False), wait=3.0)
    ev["both_spectators_actor_spectator"] = b.event("obs spectator: actor -> spectator again", lambda: sv("gamemode spectator KilnTest", show=False), wait=3.0)
    ev["obs_creative_again"] = b.event("actor spectator: obs -> creative", lambda: sv("gamemode creative Obs", show=False), wait=3.0)
    ev["a_creative"] = b.event("obs creative: actor -> creative", lambda: sv("gamemode creative KilnTest", show=False), wait=3.0)
    sv("gamemode survival KilnTest", show=False)
    return ev


def powder(b):
    ev = {}
    fill(-2, 200, -2, 2, 201, 2, "minecraft:powder_snow")
    sv("tp KilnTest 0 199.9 0", show=False)
    time.sleep(2.0)

    def walk():
        b.a.key("w", "down")
        time.sleep(3.0)
        b.a.key("w", "up")
    quiet = r"^entity_data player:\w+ [17]=-?\d+$|^SetHealth"
    ev["walk_in_powder_snow"] = b.event("walking 3 s in powder snow", walk, wait=1.0, drop_re=quiet)
    ev["stand_in_powder_snow"] = b.event("standing 2 s in powder snow", lambda: time.sleep(2.0), wait=0.5, drop_re=quiet)
    fill(-2, 200, -2, 2, 201, 2, "minecraft:air")
    return ev


def sneak(b):
    ev = {}
    time.sleep(1)
    ev["sneak_down"] = b.event("sneak down", lambda: b.a.key("shift", "down"), wait=1.0)
    ev["sneak_up"] = b.event("sneak up", lambda: b.a.key("shift", "up"), wait=1.0)
    ev["sneak_tap"] = b.event("sneak tap (short)", lambda: (b.a.key("shift", "down"), time.sleep(0.12), b.a.key("shift", "up")), wait=1.0)
    return ev


def instant(b):
    ev = {}
    sv("kill @e[type=!player]", show=False)
    ev["heal_cmd"] = b.event("/effect give instant_health", lambda: (sv("damage KilnTest 6 minecraft:generic", show=False), time.sleep(0.5), sv("effect give KilnTest minecraft:instant_health 1 0", show=False)), wait=2.0, sounds=True)
    ev["harm_cmd"] = b.event("/effect give instant_damage", lambda: sv("effect give KilnTest minecraft:instant_damage 1 0", show=False), wait=2.0, sounds=True)
    ev["both"] = b.event("/effect give instant_health + speed", lambda: (sv("effect give KilnTest minecraft:instant_health 1 0", show=False), sv("effect give KilnTest minecraft:speed 5 0", show=False)), wait=2.0)
    sv("effect clear KilnTest", show=False)
    sv("effect give KilnTest minecraft:instant_health 1 5", show=False)
    time.sleep(1.0)
    splash = '{Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:healing"}}},Motion:[0.0,-1.0,0.0]}'
    ev["splash_heal"] = b.event("splash potion of healing on the actor", lambda: (sv("damage KilnTest 6 minecraft:generic", show=False), time.sleep(0.4), sv(f"summon minecraft:splash_potion 0.0 201 0.0 {splash}", show=False)), wait=2.0, sounds=True)
    splash_h = '{Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:harming"}}},Motion:[0.0,-1.0,0.0]}'
    ev["splash_harm"] = b.event("splash potion of harming on the actor", lambda: sv(f"summon minecraft:splash_potion 0.0 201 0.0 {splash_h}", show=False), wait=2.0, sounds=True)
    splash_s = '{Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:swiftness"}}},Motion:[0.0,-1.0,0.0]}'
    ev["splash_speed"] = b.event("splash potion of swiftness (not instant) on the actor", lambda: sv(f"summon minecraft:splash_potion 0.0 201 0.0 {splash_s}", show=False), wait=2.0, sounds=True)
    sv("effect clear KilnTest", show=False)
    ling = '{Item:{id:"minecraft:lingering_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:healing"}}},Motion:[0.0,-1.0,0.0]}'
    ev["lingering_heal"] = b.event("lingering potion of healing lands next to the actor", lambda: (sv("damage KilnTest 6 minecraft:generic", show=False), time.sleep(0.4), sv(f"summon minecraft:lingering_potion 0.0 201 0.0 {ling}", show=False)), wait=5.0, sounds=True)
    sv("kill @e[type=minecraft:area_effect_cloud]", show=False)
    tipped = '{Motion:[0.0,-1.5,0.0],item:{id:"minecraft:tipped_arrow",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:healing"}}}}'
    ev["tipped_heal"] = b.event("an arrow tipped with healing hits the actor", lambda: (sv("damage KilnTest 6 minecraft:generic", show=False), time.sleep(0.4), sv(f"summon minecraft:arrow 0.2 203 0.2 {tipped}", show=False)), wait=2.5, sounds=True)
    tipped_h = '{Motion:[0.0,-1.5,0.0],item:{id:"minecraft:tipped_arrow",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:harming"}}}}'
    ev["tipped_harm"] = b.event("an arrow tipped with harming hits the actor", lambda: sv(f"summon minecraft:arrow 0.2 203 0.2 {tipped_h}", show=False), wait=2.5, sounds=True)
    # drinking: the actor holds a healing potion and drinks
    sv("clear KilnTest", show=False)
    sv('give KilnTest minecraft:potion[minecraft:potion_contents={potion:"minecraft:healing"}]', show=False)
    time.sleep(0.5)

    def drink():
        b.a.hotbar(1)
        b.a.mouse("right", "down")
        time.sleep(2.2)
        b.a.mouse("right", "up")
    ev["drink_heal"] = b.event("drinks a potion of healing", lambda: (sv("damage KilnTest 6 minecraft:generic", show=False), time.sleep(0.4), drink()), wait=1.5, sounds=True)
    return ev


SCENARIOS = {"arrows": arrows, "sting": sting, "spectator": spectator, "powder": powder, "sneak": sneak, "instant": instant}

if __name__ == "__main__":
    what = sys.argv[1]
    b = Both()
    names = list(SCENARIOS) if what == "all" else [what]
    for n in names:
        setup(b)
        res = SCENARIOS[n](b)
        p = OUT / f"{n}_{T}.json"
        p.write_text(json.dumps(res, indent=1))
        print("wrote", p)
