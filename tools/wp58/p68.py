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
