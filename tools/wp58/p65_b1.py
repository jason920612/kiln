"""wp65 batch 1: multiplayer interactions with two real clients (A = KilnTest, B = Obs), the same script against Kiln (TARGET=K)
and the vanilla reference server (TARGET=V).   usage: p65_b1.py pvp|chat|drop|boat|sleep ...
Facts ("F: ...") are written to ~/wp65/b1_<names>_<K|V>.json for a diff."""
from p65 import *
import sys

a, b = clients()
which = sys.argv[1:] or ["pvp"]
FACTS = []


def F(*x):
    s = " ".join(str(i) for i in x)
    FACTS.append(s)
    print("F:", s, flush=True)


def base(mode="survival"):
    if T == "V":
        sv("op KilnTest", show=False)
        sv("op Obs", show=False)
    alive(a)
    alive(b)
    sv("gamerule spawn_mobs false", show=False)
    sv("difficulty normal", show=False)
    sv("kill @e[type=!player]", show=False)
    sv("time set day", show=False)
    sv("weather clear", show=False)
    platform(-20, -20, 20, 20, y=199, clear=12)
    for p in ("KilnTest", "Obs"):
        sv(f"gamemode {mode} {p}", show=False)
        sv(f"clear {p}", show=False)
        sv(f"effect clear {p}", show=False)
    sv("tp KilnTest 0 200 0", show=False)
    sv("tp Obs 0 200 2.5", show=False)
    time.sleep(2.5)


def hp(c):
    return c.state(0)["health"]


def face(h=1.2):
    sa = a.state(0)
    sb0 = b.state(0)
    if math.hypot(sa["x"] - sb0["x"], sa["z"] - sb0["z"]) > 2.3:
        b.walk_to(sa["x"], sa["z"], timeout=3, tol=1.6)
    sa = a.state(0)
    sb_ = b.state(0)
    a.lookat(sb_["x"], sb_["y"] + h, sb_["z"])
    b.lookat(sa["x"], sa["y"] + h, sa["z"])


def strike(label, wait=2.0, jump=False):
    """B hits A once (B walks up to A and aims first); returns A's damage and whether the crosshair was on A."""
    face()
    time.sleep(wait)
    face()
    time.sleep(0.4)
    onA = (b.state(0).get("hit") or {}).get("type") == "entity"
    h0 = hp(a)
    if jump:
        b.key("space", "down")
        time.sleep(0.35)
        b.key("space", "up")
        time.sleep(0.2)
        face()
    b.mouse("left", "tap")
    time.sleep(1.0)
    d = round(h0 - hp(a), 3)
    F(label, "damage", d, "aimed", onA)
    return d


def strike(label, wait=2.0, jump=False):
    """B hits A once (B walks up to A and aims first); returns A's damage and whether the crosshair was on A."""
    face()
    time.sleep(wait)
    face()
    time.sleep(0.4)
    onA = (b.state(0).get("hit") or {}).get("type") == "entity"
    h0 = hp(a)
    if jump:
        b.key("space", "down")
        time.sleep(0.35)
        b.key("space", "up")
        time.sleep(0.2)
        face()
    b.mouse("left", "tap")
    time.sleep(1.0)
    d = round(h0 - hp(a), 3)
    F(label, "damage", d, "aimed", onA)
    return d


def pvp():
    base()
    sv("give KilnTest minecraft:shield 1", show=False)
    sv("give KilnTest minecraft:iron_chestplate 1", show=False)
    sv("give Obs minecraft:diamond_axe 1", show=False)
    sv("give Obs minecraft:diamond_sword 1", show=False)
    time.sleep(1)
    a.select("minecraft:iron_chestplate")
    a.mouse("right", "tap")
    time.sleep(0.6)
    a.select("minecraft:shield")
    a.key("f")
    time.sleep(0.6)
    pa = eid_of(b)
    e = ent(b, pa)
    F("B sees A gear:", e["main"], e["off"], e["chest"])
    b.select("minecraft:diamond_sword")
    time.sleep(1.0)
    strike("sword, A unblocked", 3.0)
    sv("effect give KilnTest minecraft:instant_health 1 5", show=False)
    time.sleep(2.0)
    a.mouse("right", "down")
    time.sleep(1.2)
    e = ent(b, pa)
    F("shield up: B sees using/blocking", e["using"], e["blocking"])
    strike("sword vs raised shield", 2.0)
    b.select("minecraft:diamond_axe")
    time.sleep(1.0)
    strike("axe vs raised shield", 2.5)
    time.sleep(0.5)
    e = ent(b, pa)
    F("after the axe: B sees A using/blocking:", e["using"], e["blocking"], "A client using:", a.state(0).get("using"))
    a.mouse("right", "up")
    time.sleep(0.5)
    shot("pvp1")
    a.mouse("right", "down")
    time.sleep(0.8)
    e = ent(b, pa)
    F("shield on cooldown, A raises: B sees using/blocking", e["using"], e["blocking"])
    strike("axe vs shield on cooldown (held up)", 2.5)
    a.mouse("right", "up")
    time.sleep(1.0)
    strike("axe, shield down", 2.5)
    sv("effect give KilnTest minecraft:instant_health 1 5", show=False)
    time.sleep(3.5)
    b.select("minecraft:diamond_sword")
    time.sleep(1.0)
    strike("sword jump crit", 2.5, jump=True)
    # the shield off cooldown: the disabled shield works again after 5 s (axe: 100 ticks)
    time.sleep(6.0)
    a.mouse("right", "down")
    time.sleep(1.2)
    strike("sword vs shield after the cooldown", 1.0)
    a.mouse("right", "up")


def chatt():
    base()
    chat()
    a.say("hello from A")
    time.sleep(1.0)
    b.say("hi A, B here")
    time.sleep(1.0)
    F("log lines", [l for l in chat() if "hello" in l or "hi A" in l])
    sv("say server says hi", show=False)
    sv("msg Obs secret", show=False)
    time.sleep(0.6)
    F("log after say/msg", [l for l in chat() if "server" in l.lower() or "secret" in l])


def dropt():
    base()
    sv("give KilnTest minecraft:cobblestone 40", show=False)
    time.sleep(1)
    a.select("minecraft:cobblestone")
    sv("tp Obs 0 200 6", show=False)
    time.sleep(1.5)
    a.look(0, 0)
    a.cmd("key q tap 2")
    t_drop = time.time()
    time.sleep(0.2)
    for i in range(30):
        its = [e for e in b.state(10)["entities"] if e["t"] == "item"]
        if i in (0, 10):
            F("item seen by B", [(e["item"], e["n"]) for e in its])
        time.sleep(0.1)
    e = [e for e in b.state(10)["entities"] if e["t"] == "item"]
    if e:
        b.walk_to(e[0]["x"], e[0]["z"], timeout=6, tol=0.3)
    time.sleep(1.5)
    F("B inv after", b.inv(), "A inv", a.inv())
    sv("give KilnTest minecraft:stick 5", show=False)
    time.sleep(0.5)
    a.select("minecraft:stick")
    a.key("q")
    time.sleep(0.3)
    F("A inv right after Q (one stick dropped)", a.inv())


def boat():
    base()
    sv("fill -4 198 -4 4 199 4 minecraft:stone", show=False)
    sv("fill -4 199 -4 4 199 4 minecraft:water", show=False)
    sv("tp KilnTest 0 200 -6", show=False)
    sv("tp Obs 1 200 -6", show=False)
    time.sleep(1.5)
    sv("summon minecraft:oak_boat 0 199.5 0", show=False)
    time.sleep(1.0)
    bo = [e for e in a.state(15)["entities"] if "boat" in e["t"]]
    F("boat seen", len(bo), bo[0]["t"] if bo else None)
    bid = bo[0]["id"]

    def ride(c):
        for _ in range(4):
            if c.state(0)["vehicle"] is not None:
                return True
            st = c.state(8)
            e = [e for e in st["entities"] if e["id"] == bid][0]
            c.walk_to(e["x"], e["z"], timeout=4, tol=1.2)
            c.lookat(e["x"], e["y"] + 0.4, e["z"])
            time.sleep(0.5)
            c.mouse("right", "tap")
            time.sleep(0.8)
        return c.state(0)["vehicle"] is not None

    F("A rides", ride(a))
    F("B rides", ride(b))
    e = ent(a, bid)
    F("boat passengers count", len(e["passengers"]))
    time.sleep(1)
    x0 = (a.state(0)["x"], a.state(0)["z"])
    a.key("w", "down")
    time.sleep(3.0)
    a.key("w", "up")
    time.sleep(0.8)
    xa = (a.state(0)["x"], a.state(0)["z"])
    xb = (b.state(0)["x"], b.state(0)["z"])
    F("A moved", round(math.hypot(xa[0] - x0[0], xa[1] - x0[1]), 1), "B distance to A", round(math.hypot(xa[0] - xb[0], xa[1] - xb[1]), 2))
    shot("boat2")
    b.key("shift", "tap")
    time.sleep(0.8)
    F("B dismounted", b.state(0)["vehicle"] is None, "boat passengers", len(ent(a, bid)["passengers"]))


def sleept():
    base()
    sv("setblock 6 200 0 minecraft:red_bed[facing=east,part=foot]", show=False)
    sv("setblock 7 200 0 minecraft:red_bed[facing=east,part=head]", show=False)
    sv("setblock -6 200 0 minecraft:blue_bed[facing=west,part=foot]", show=False)
    sv("setblock -7 200 0 minecraft:blue_bed[facing=west,part=head]", show=False)
    sv("time set night", show=False)
    time.sleep(0.5)
    F("night time", sv("time query time", show=False).strip()[-30:])
    chat()
    a.walk_to(5.5, 0.5, timeout=6, tol=0.8)
    a.lookat(6.5, 200.3, 0.5)
    time.sleep(0.6)
    a.mouse("right", "tap")
    time.sleep(2.0)
    s = a.state(0)
    F("A sleeping", s["sleeping"], "overlay", s["overlay"])
    time.sleep(6.0)
    F("after 6 s with A asleep (B awake) time", sv("time query time", show=False).strip()[-30:])
    F("chat", chat())
    sv("gamerule players_sleeping_percentage 50", show=False)
    time.sleep(8.0)
    F("percentage 50 -> time", sv("time query time", show=False).strip()[-30:], "A sleeping", a.state(0)["sleeping"])
    F("chat", chat())
    sv("gamerule players_sleeping_percentage 100", show=False)


NAMES = {"pvp": pvp, "chat": chatt, "drop": dropt, "boat": boat, "sleep": sleept}
for w in which:
    print("=====", w)
    NAMES[w]()
json.dump(FACTS, open(OUT / f"b1_{'_'.join(which)}_{T}.json", "w"), indent=1)
