"""Python side of the wp58 client driver (tools/wp58/ClientCtl.java, loaded as a java agent).

Usable as a module (`from ctl import C; c = C()`) or a CLI: python tools/wp58/ctl.py state
"""
import json
import math
import socket
import subprocess
import sys
import time
from pathlib import Path

# Runs on the VM next to the server (~/wp58) and the real client (Xvfb :99, ~/mc-client).
HOME = Path.home()
SHOTS = HOME / "wp58" / "shots"


def server(cmd):
    """Run a console command on the wp58 server (its console is the fifo ~/wp58/console)."""
    with open(HOME / "wp58" / "console", "w") as f:
        f.write(cmd + chr(10))


def server_log(n=40, pattern="-v 'chunk totals|mspt mean'"):
    return subprocess.run(f"grep {pattern} ~/wp58/server.log | tail -{n}", shell=True, capture_output=True, text=True).stdout


class C:
    def __init__(self, port=25999):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=15)
        self.f = self.s.makefile("rw", encoding="utf-8", newline="\n")

    def cmd(self, line):
        self.f.write(line + "\n")
        self.f.flush()
        r = self.f.readline().strip()
        if r.startswith("ERR"):
            raise RuntimeError(r)
        return r

    def state(self, rng=24):
        return json.loads(self.cmd(f"state {rng}"))

    ALIAS = {"ctrl": "LCONTROL", "shift": "LSHIFT", "enter": "RETURN", "esc": "ESCAPE", "slash": "SLASH"}

    def key(self, name, mode="tap"):
        name = self.ALIAS.get(name, name)
        return self.cmd(f"key {name} {mode}")

    def hold(self, name, seconds):
        self.key(name, "down")
        time.sleep(seconds)
        self.key(name, "up")

    def mouse(self, b, mode="tap"):
        return self.cmd(f"mouse {b} {mode}")

    def look(self, yaw, pitch):
        return self.cmd(f"look {yaw} {pitch}")

    def lookat(self, x, y, z):
        return self.cmd(f"lookat {x} {y} {z}")

    def hotbar(self, n):
        return self.cmd(f"hotbar {n}")

    def say(self, text):
        return self.cmd("say " + text)

    def find(self, sub, r=16, n=40):
        return json.loads(self.cmd(f"find {sub} {r} {n}"))

    def block(self, x, y, z):
        return self.cmd(f"block {x} {y} {z}")

    _last_click = (None, 0.0)

    def guiclick(self, slot, button="left", shift=False, wait=0.12):
        # two clicks on one slot within ~250 ms are the client's double click (collect all)
        if self._last_click[0] == slot:
            time.sleep(max(0.0, 0.32 - (time.time() - self._last_click[1])))
        self._last_click = (slot, time.time())
        r = self.cmd(f"guiclick {slot} {button}" + (" shift" if shift else ""))
        time.sleep(wait)  # the server answers every click; later clicks must see its slot updates
        return r

    def inv(self):
        st = self.state()
        d = {}
        for e in st.get("inv", []):
            d[e["id"]] = d.get(e["id"], 0) + e["n"]
        return d

    def release(self):
        return self.cmd("releaseall")

    def walk_to(self, x, z, timeout=30, sprint=False, tol=0.7):
        """Face (x, z) and walk with w, jumping when stuck. Returns True when within tol."""
        t0 = time.time()
        last = None
        stuck = 0
        self.key("w", "down")
        if sprint:
            self.key("ctrl", "down")
        try:
            while time.time() - t0 < timeout:
                st = self.state(0)
                dx, dz = x - st["x"], z - st["z"]
                if math.hypot(dx, dz) < tol:
                    return True
                self.look(math.degrees(math.atan2(-dx, dz)), 5)
                pos = (round(st["x"], 1), round(st["z"], 1))
                stuck = stuck + 1 if pos == last else 0
                last = pos
                if stuck >= 2:
                    self.key("space", "tap")
                    stuck = 0
                time.sleep(0.15)
            return False
        finally:
            self.key("w", "up")
            if sprint:
                self.key("ctrl", "up")


if __name__ == "__main__":
    c = C()
    print(c.cmd(" ".join(sys.argv[1:])))


def _mine(self, x, y, z, timeout=20, stand=True):
    """Look at the centre of a block and hold attack until it is gone."""
    t0 = time.time()
    self.mouse("left", "down")
    try:
        while time.time() - t0 < timeout:
            if self.block(x, y, z).startswith("Block{minecraft:air}") or "air" in self.block(x, y, z).split("[")[0]:
                return True
            self.lookat(x + 0.5, y + 0.5, z + 0.5)
            time.sleep(0.1)
        return False
    finally:
        self.mouse("left", "up")


def _near(self, x, y, z, dist=3.0, timeout=20):
    """Walk until the eye is within dist of the block centre horizontally."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        st = self.state(0)
        d = math.hypot(st["x"] - (x + 0.5), st["z"] - (z + 0.5))
        if d <= dist:
            return True
        self.walk_to(x + 0.5, z + 0.5, timeout=3, tol=dist)
    return False


C.mine = _mine
C.near = _near


def shot(name):
    """Screenshot of the Xvfb display (the client's own virtual screen) to ~/wp58/shots/<name>.png."""
    SHOTS.mkdir(parents=True, exist_ok=True)
    out = SHOTS / f"{name}.png"
    subprocess.run(["import", "-display", ":99", "-window", "root", str(out)], check=True)
    return out
