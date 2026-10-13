"""wp60 helpers on top of ctl.py (runs on the VM next to the server and the real client: KILN_PLAY_DIR=wp60 PYTHONPATH=tools/wp58 python3 script.py)."""
import json, math, re, time, subprocess, os
from pathlib import Path
from ctl import *
import ctl

LOG = PLAY / "server.log"
CLOG = HOME / "mc-client" / "game" / "stdout.log"
ANSI = re.compile(r"\x1b\[[0-9;]*m")

def _logsize():
    return LOG.stat().st_size

def sv(cmd, wait=0.6, show=True):
    """Run a console command; return the server log text it produced (no periodic reports)."""
    n = _logsize()
    server(cmd)
    time.sleep(wait)
    with open(LOG, "rb") as f:
        f.seek(n)
        txt = f.read().decode(errors="replace")
    lines = [ANSI.sub("", l) for l in txt.splitlines() if "chunk totals" not in l and "mspt mean" not in l]
    lines = [re.sub(r"^\S+Z\s+", "", l) for l in lines]
    out = "\n".join(lines)
    if show and out:
        print(out[:1500])
    return out

def svlog_warn(since=[0]):
    """WARN/ERROR lines in the server log since last call."""
    with open(LOG, "rb") as f:
        f.seek(since[0])
        txt = f.read().decode(errors="replace")
        since[0] = f.tell()
    return [ANSI.sub("", l) for l in txt.splitlines() if re.search(r"\b(WARN|ERROR)\b", ANSI.sub("", l))]

def clog_warn(since=[0]):
    if not CLOG.exists():
        return []
    with open(CLOG, "rb") as f:
        f.seek(since[0])
        txt = f.read().decode(errors="replace")
        since[0] = f.tell()
    ok = ("BackendCreation", "authlib", "Narrator", "flite", "Suppressed", "	at ", "Realms", "user properties", "narrator", "backend OpenGL", "end_of_frame", "401", "Caused by: MinecraftClientHttp")
    return [l for l in txt.splitlines() if re.search(r"/(WARN|ERROR)\]|Exception|decode", l) and not any(k in l for k in ok)][:30]

def chat(since=[0], keep=False):
    """New [CHAT] lines of the client log since the last call."""
    with open(CLOG, "rb") as f:
        f.seek(since[0])
        txt = f.read().decode(errors="replace")
        since[0] = f.tell()
    return [l.split("[CHAT] ", 1)[1] for l in txt.splitlines() if "[CHAT]" in l]

def fill(x0, y0, z0, x1, y1, z1, blk, mode=""):
    return sv(f"fill {x0} {y0} {z0} {x1} {y1} {z1} {blk} {mode}".strip(), show=False)

def sb(x, y, z, blk):
    return sv(f"setblock {x} {y} {z} {blk}", show=False)

def blk(c, x, y, z):
    return c.block(x, y, z).replace("Block{minecraft:", "").replace("}", "", 1)

def me(c):
    s = c.state(0)
    return round(s["x"], 2), round(s["y"], 2), round(s["z"], 2)

def tpme(x, y, z, dim=None):
    if dim:
        server(f"execute in {dim} run tp KilnTest {x} {y} {z}")
    else:
        server(f"tp KilnTest {x} {y} {z}")
    time.sleep(2.5)

def invc(c):
    return c.inv()

def entities(c, rng=24, t=None):
    es = c.state(rng)["entities"]
    return [e for e in es if t is None or e["t"] == t]

def shotn(name):
    return str(ctl.shot(name))

def ent_by(c, t, rng=24):
    return [e for e in c.state(rng)["entities"] if e["t"] == t]

def interact_ent(c, eid, button="right", tries=6, dy=0.6):
    """Look at entity eid and click until the crosshair is on it."""
    for _ in range(tries):
        st = c.state(12)
        e = next((e for e in st["entities"] if e["id"] == eid), None)
        if e is None:
            return False
        d = math.hypot(e["x"] - st["x"], e["z"] - st["z"])
        if d > 2.6:
            c.walk_to(e["x"], e["z"], timeout=1.5, tol=2.0)
        c.lookat(e["x"], e["y"] + dy, e["z"])
        time.sleep(0.4)
        h = c.state(0).get("hit")
        if h and h.get("type") == "entity" and h["eid"] == eid:
            c.mouse(button, "tap")
            time.sleep(0.3)
            return True
    return False

def kill_nearest(c, t, rng=60, timeout=40, dy=0.5):
    """Walk to the nearest entity of type t and hit it until it is gone. Returns True if one was killed."""
    t0 = time.time()
    tid = None
    while time.time() - t0 < timeout:
        st = c.state(rng)
        es = [e for e in st["entities"] if e["t"] == t]
        if tid is not None:
            es = [e for e in es if e["id"] == tid]
            if not es:
                return True
        if not es:
            return False
        e = min(es, key=lambda e: (e["x"]-st["x"])**2 + (e["z"]-st["z"])**2) if tid is None else es[0]
        tid = e["id"]
        d = math.hypot(e["x"]-st["x"], e["z"]-st["z"])
        if d > 2.4:
            c.walk_to(e["x"], e["z"], timeout=2.5, tol=1.8)
            continue
        c.lookat(e["x"], e["y"]+dy, e["z"])
        time.sleep(0.15)
        c.mouse("left", "tap")
        time.sleep(0.55)
    return False

def fish(c, n, rate=40, maxwait=45):
    """Cast and reel n times; returns [(hooked, item deltas)] per cast."""
    results = []
    sv(f"tick rate {rate}", show=False)
    def bobber():
        b = [e for e in c.state(10)["entities"] if e["t"] == "fishing_bobber"]
        return b[0] if b else None
    if bobber():
        c.mouse("right", "tap"); time.sleep(1.0)
    for k in range(n):
        before = c.inv()
        c.lookat(*FISH_AT)
        c.mouse("right", "tap"); time.sleep(0.8)
        t0 = time.time(); base = -1e9; hooked = False
        while time.time() - t0 < maxwait:
            b = bobber()
            if b is None:
                break
            y = b["y"]
            if time.time() - t0 < 2.0:
                base = max(base, y)
            elif y < base - 0.3:
                hooked = True
                break
            time.sleep(0.03)
        c.mouse("right", "tap"); time.sleep(1.2)
        c.collect(6, 3)
        after = c.inv()
        d = {k2: after.get(k2, 0) - before.get(k2, 0) for k2 in set(after) | set(before) if after.get(k2, 0) != before.get(k2, 0)}
        results.append((hooked, d))
    sv("tick rate 20", show=False)
    return results
FISH_AT = (-31, 200.5, -31)

def alive(c):
    for _ in range(4):
        if c.screen() != "DeathScreen":
            break
        c.cmd("mousepos 480 298"); time.sleep(1.5)
        c.mouse("left", "tap"); time.sleep(2.5)
    return c.state(0)["health"]

RAIDERS = ("pillager", "vindicator", "ravager", "evoker", "witch", "vex", "illusioner")

def heal(c):
    if c.screen() is not None:
        return
    h = c.state(0)["health"]
    if h < 9:
        c.select("minecraft:golden_apple")
        c.mouse("right", "down"); time.sleep(2.0); c.mouse("right", "up")
        c.select("minecraft:diamond_sword")

def clear_raiders(c, timeout=300, log=None):
    t0 = time.time(); kills = 0
    while time.time() - t0 < timeout:
        alive(c)
        server("effect give KilnTest minecraft:resistance 60 3 true")
        heal(c)
        es = [e for e in c.state(100)["entities"] if e["t"] in RAIDERS]
        if not es:
            return kills
        st = c.state(0)
        e = min(es, key=lambda e: (e["x"]-st["x"])**2 + (e["z"]-st["z"])**2)
        c.select("minecraft:diamond_sword")
        if kill_nearest(c, e["t"], rng=100, timeout=25, dy=0.9):
            kills += 1
            if log is not None: log.append(e["t"])
    return kills

def is_air(b):
    return b in ("air", "cave_air", "void_air")

def free_spot(c, x0, y0, z0, r=12, dy=8, need_ground=True):
    best = None
    for dx in range(-r, r + 1):
        for dz in range(-r, r + 1):
            x, z = x0 + dx, z0 + dz
            for y in range(max(-60, y0 - dy), y0 + dy + 1):
                if is_air(blk(c, x, y, z)) and is_air(blk(c, x, y + 1, z)):
                    below = blk(c, x, y - 1, z)
                    if need_ground and (is_air(below) or "lava" in below or "water" in below or "fire" in below):
                        continue
                    d = abs(dx) + abs(dz) + abs(y - y0)
                    if best is None or d < best[0]:
                        best = (d, x, y, z)
    return best


# ---- the vanilla reference server (tools/wp58/wp60_vanilla.sh): console fifo and log in ~/wp60/vanilla ----
VDIR = HOME / "wp60" / "vanilla"

def vsv(cmd, wait=0.8, show=True):
    """Run a command on the vanilla reference server; return the log lines it produced."""
    log = VDIR / "server.log"
    n = log.stat().st_size
    with open(VDIR / "console", "w") as f:
        f.write(cmd + chr(10))
    time.sleep(wait)
    with open(log, "rb") as f:
        f.seek(n)
        txt = f.read().decode(errors="replace")
    out = "\n".join(re.sub(r"^\[[0-9:]+\] \[[^\]]*\]: ", "", l) for l in txt.splitlines())
    if show and out:
        print(out[:1500])
    return out
