"""wp68 helpers: two real clients (A = KilnTest agent 25999, B = Obs agent 26000) on the Kiln (or vanilla) server."""
import json, math, os, re, sys, time
from pathlib import Path
import p61
from p61 import *

p61.VDIR = HOME / "wp68" / "vanilla"
T = os.environ.get("TARGET", "K")
OUT = HOME / "wp68"

def clients():
    return C(25999), C(26000)

def both_survival(x=0, y=200, z=0):
    sv("gamemode survival KilnTest", show=False); sv("gamemode survival Obs", show=False)
    sv(f"tp KilnTest {x} {y} {z}", show=False); sv(f"tp Obs {x+3} {y} {z+3}", show=False)
    time.sleep(2.5)

def ent(c, eid):
    return json.loads(c.cmd(f"ent {eid}"))

def eid_of(c, t="player", near=None):
    es = [e for e in c.state(400)["entities"] if e["t"] == t]
    return es[0]["id"] if es else None

def say(*a):
    print(*a, flush=True)

def mspt():
    """last 'mspt mean' line of the server log"""
    txt = ANSI.sub("", LOG.read_text(errors="replace"))
    ls = [l for l in txt.splitlines() if "mspt mean" in l]
    return re.sub(r"^\S+Z\s+\S+\s+\S+\s+", "", ls[-1])[:140] if ls else ""

def chunks():
    txt = ANSI.sub("", LOG.read_text(errors="replace"))
    ls = [l for l in txt.splitlines() if "chunk totals" in l]
    return ls[-1].split("chunk totals:")[1][:330] if ls else ""


def go(c, tx, tz, timeout=120, sprint=True, tol=2.0, log=None, dig=True):
    """Walk (sprinting) towards (tx, tz); when stuck, mine the blocks in front (feet and head) or jump. Returns True on arrival."""
    t0 = time.time()
    last = None
    stuck_n = 0
    c.key("w", "down")
    if sprint:
        c.key("ctrl", "down")
    try:
        while time.time() - t0 < timeout:
            st = c.state(0)
            if st["health"] <= 0 or c.screen() == "DeathScreen":
                return False
            dx, dz = tx - st["x"], tz - st["z"]
            if math.hypot(dx, dz) < tol:
                return True
            yaw = math.degrees(math.atan2(-dx, dz))
            c.look(yaw, 10)
            p = (round(st["x"], 1), round(st["z"], 1))
            stuck_n = stuck_n + 1 if p == last else 0
            last = p
            if log is not None and int(time.time() - t0) % 15 == 0:
                log.append((round(time.time() - t0), p))
            if stuck_n >= 3:
                # blocked: mine the two blocks ahead (feet and head level) else jump
                ux, uz = dx / max(1e-6, math.hypot(dx, dz)), dz / max(1e-6, math.hypot(dx, dz))
                bx, bz = math.floor(st["x"] + ux * 0.9), math.floor(st["z"] + uz * 0.9)
                by = math.floor(st["y"])
                c.key("w", "up")
                for yy in (by + 1, by):
                    b_ = c.block(bx, yy, bz)
                    if "air" not in b_.split("[")[0] and "lava" not in b_ and "portal" not in b_ and "fire" not in b_:
                        if dig:
                            c.mine(bx, yy, bz, timeout=6)
                c.key("space", "tap")
                c.key("w", "down")
                stuck_n = 0
            time.sleep(0.4)
        return False
    finally:
        c.key("w", "up")
        if sprint:
            c.key("ctrl", "up")


def alive_b(c):
    """`alive` for the second client (480x270 window: its Respawn button is at (240, 149))."""
    for _ in range(4):
        if c.screen() != "DeathScreen":
            break
        c.cmd("mousepos 240 149"); time.sleep(1.5)
        c.mouse("left", "tap"); time.sleep(2.5)
    return c.state(0)["health"]


# ---- wp68: facts and packet recordings of one scenario, written per target so K and V can be diffed ----
import collections
SPYDIR = OUT / "spy"
FACTDIR = OUT / "facts"
_NUM = re.compile(r"-?\d+\.\d+(?:E-?\d+)?")
_facts_name = [None]
FACTS = []

def scenario(name):
    """Start a scenario: facts are collected into OUT/facts/<name>_<T>.txt"""
    _facts_name[0] = name
    FACTS.clear()
    FACTDIR.mkdir(parents=True, exist_ok=True)
    SPYDIR.mkdir(parents=True, exist_ok=True)

def F(*x):
    s = " ".join(str(i) for i in x)
    FACTS.append(s)
    print("F:", s, flush=True)
    if _facts_name[0]:
        (FACTDIR / f"{_facts_name[0]}_{T}.txt").write_text("\n".join(FACTS) + "\n")

def rec_start(c):
    c.cmd("spy on")
    c.cmd("spy clear")
    c.cmd("spylog clear")

def rec_stop(c, tag, show=True):
    """Save the packets since rec_start as OUT/spy/<scenario>-<tag>_<T>.json and print the counts of normalised lines."""
    lines = json.loads(c.cmd("spylog clear"))
    norm = [_NUM.sub("#", l) for l in lines]
    cnt = collections.Counter(norm)
    nm = f"{_facts_name[0]}-{tag}"
    (SPYDIR / f"{nm}_{T}.json").write_text(json.dumps(dict(cnt), indent=0, sort_keys=True))
    if show:
        print(f"spy {tag}: {len(lines)} lines, {len(cnt)} distinct", flush=True)
    return cnt

def spy_diff(name, tag):
    """Differences between the Kiln (K) and vanilla (V) recordings of <name>-<tag>"""
    k = json.loads((SPYDIR / f"{name}-{tag}_K.json").read_text())
    v = json.loads((SPYDIR / f"{name}-{tag}_V.json").read_text())
    out = []
    for key in sorted(set(k) | set(v)):
        if k.get(key, 0) != v.get(key, 0):
            out.append(f"K{k.get(key,0):>4} V{v.get(key,0):>4}  {key[:200]}")
    return out

def pos(c):
    s = c.state(0)
    return round(s["x"], 2), round(s["y"], 2), round(s["z"], 2)

def dist(p, q):
    return round(math.hypot(p[0] - q[0], p[2] - q[2]), 2)

def give(nm, item, n=1):
    sv(f"give {nm} {item} {n}", show=False)

def kit(nm, extra=()):
    sv(f"clear {nm}", show=False)
    sv(f"effect clear {nm}", show=False)
    sv(f"gamemode survival {nm}", show=False)
    for it in extra:
        sv(f"give {nm} {it}", show=False)

def clear_area():
    """Remove every non-player entity (dying slimes and sulfur cubes split, so repeat)."""
    for _ in range(4):
        sv("kill @e[type=!player]", wait=0.35, show=False)

def select_item(c, item):
    return c.select(item)

def use_on_entity(c, eid, item, button="right", dy=0.6, tries=8):
    """Hold `item`, look at entity eid and right-click it (real interaction). True when the click landed on the entity."""
    c.select(item)
    return interact_ent(c, eid, button=button, tries=tries, dy=dy)

def ents_of(c, t, rng=30):
    return [e for e in c.state(rng)["entities"] if e["t"] == t]


# ---- wp68: two real clients ----
def two():
    return C(25999), C(26000)

def setup_two(x=20.5, y=200, z=20.5, mode="survival", resist=True):
    """Platform 0..40, both players in survival with resistance (so a stray mob does not end the run)."""
    if T == "V":
        sv("op KilnTest", show=False)
        sv("op Obs", show=False)
    sv("gamerule spawn_mobs false", show=False)
    sv("gamerule spawn_phantoms false", show=False)
    sv("gamerule mob_griefing true", show=False)
    sv("difficulty normal", show=False)
    sv("time set day", show=False)
    sv("weather clear", show=False)
    platform(0, 0, 40, 40, y=199, clear=40)
    for nm in ("KilnTest", "Obs"):
        sv(f"clear {nm}", show=False)
        sv(f"effect clear {nm}", show=False)
        sv(f"gamemode {mode} {nm}", show=False)
        if resist:
            sv(f"effect give {nm} minecraft:resistance 99999 4 true", show=False)
    sv(f"tp KilnTest {x} {y} {z}", show=False)
    sv(f"tp Obs {x+2} {y} {z}", show=False)
    time.sleep(2.5)

def alive2(a, b):
    alive(a)
    alive_b(b)


def alive_auto(c):
    """Respawn if dead: the Respawn button of the 960x540 actor window, or of the small 480x270 one (agent 26000)."""
    small = int(os.environ.get("KILN_AGENT_PORT", "25999")) == 26000
    for _ in range(4):
        if c.screen() != "DeathScreen":
            break
        c.cmd("mousepos 240 149" if small else "mousepos 480 298")
        time.sleep(1.5)
        c.mouse("left", "tap")
        time.sleep(2.5)
    return c.state(0)["health"]
