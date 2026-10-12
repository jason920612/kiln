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


def _collect(self, rng=8, timeout=25):
    """Walk over dropped items within rng blocks until none is left."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        st = self.state(rng)
        items = [e for e in st["entities"] if e["t"] == "item"]
        if not items:
            return True
        e = min(items, key=lambda e: (e["x"] - st["x"]) ** 2 + (e["z"] - st["z"]) ** 2)
        self.walk_to(e["x"], e["z"], timeout=4, tol=0.4)
        time.sleep(0.2)
    return False


def _screen(self):
    return self.state(0).get("screen")


def _open_inv(self):
    if self.screen() != "InventoryScreen":
        self.key("e")
        time.sleep(0.4)
    return self.state(0)


def _close(self):
    for _ in range(3):
        if self.screen() is None:
            return
        self.key("esc")
        time.sleep(0.3)


def _pinv_slots(slots, menu="CraftingMenu"):
    """Menu slot indices of the player's inventory part (27 main + 9 hotbar) of a menu slot list."""
    n = len(slots)
    off = 1 if menu == "InventoryMenu" else 0  # the inventory menu has the offhand slot at the end
    return n - 36 - off, n - off


def _craft(self, grid, times=1, width=3, result=0, first=1):
    """grid: {cell_index (row-major in `width`): item id}. Put `times` of each ingredient in its
    cell, shift-click the result, close the screen."""
    st = self.state(0)
    slots = st["slots"]
    lo, hi = _pinv_slots(slots, st.get("menu"))
    for cell, item in grid.items():
        for _ in range(times):
            src = next(s for s in slots[lo:hi] if s["id"] == item and s["n"] > 0)
            self.guiclick(src["i"], "left")
            self.guiclick(first + cell, "right")
            self.guiclick(src["i"], "left")
            slots = self.state(0)["slots"]
    time.sleep(0.3)
    self.guiclick(result, "left", shift=True, wait=0.4)
    self.close()


P = "minecraft:oak_planks"
S = "minecraft:stick"


def grid(rows, key):
    """rows: list of 3-char strings (' ' empty); key: {char: item} -> {cell: item}"""
    return {r * 3 + q: key[ch] for r, row in enumerate(rows) for q, ch in enumerate(row) if ch != " "}


def _craft_table(self, table, rows, key, times=1):
    """Open the crafting table at `table` (x, y, z) if its screen is not open, craft a 3x3 recipe."""
    if self.screen() != "CraftingScreen":
        self.close()
        self.use_on(*table)
        time.sleep(0.6)
    assert self.screen() == "CraftingScreen", self.screen()
    self.craft(grid(rows, key), times=times, width=3, first=1)


def _craft2(self, rows, key, times=1):
    """A 2x2 recipe in the inventory (rows of 2 chars)."""
    self.open_inv()
    self.craft({r * 2 + q: key[ch] for r, row in enumerate(rows) for q, ch in enumerate(row) if ch != " "},
               times=times, width=2, first=1)


def _select(self, item):
    """Put `item` in the selected hotbar slot (swapping it in from the main inventory if needed)."""
    st = self.state(0)
    for e in st["inv"]:
        if e["id"] == item and e["slot"] < 9:
            self.hotbar(e["slot"] + 1)
            time.sleep(0.15)
            return True
    for e in st["inv"]:
        if e["id"] == item:
            self.open_inv()
            sel = st["selected"]
            self.guiclick(e["slot"], "left")
            self.guiclick(36 + sel, "left")
            self.guiclick(e["slot"], "left")
            self.close()
            return True
    return False


def _aim(self, x, y, z, fx=0.5, fy=0.5, fz=0.5, wait=0.25):
    self.lookat(x + fx, y + fy, z + fz)
    time.sleep(wait)
    return self.state(0).get("hit")


def _use_on(self, x, y, z, face_offset=(0.5, 0.5, 0.5), button="right"):
    h = self.aim(x, y, z, *face_offset)
    self.mouse(button, "tap")
    time.sleep(0.25)
    return h


def _place(self, item, x, y, z, face="up"):
    """Place `item` against the face of block (x, y, z): aim at the face's centre."""
    self.select(item)
    off = {"up": (0.5, 0.95, 0.5), "north": (0.5, 0.5, 0.05), "south": (0.5, 0.5, 0.95),
           "east": (0.95, 0.5, 0.5), "west": (0.05, 0.5, 0.5), "down": (0.5, 0.05, 0.5)}[face]
    return self.use_on(x, y, z, off)


def _fight(self, eid, timeout=20, reach=3.0):
    """Face entity `eid` and hit it (about every 0.6 s) until it is gone."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        st = self.state(10)
        e = next((e for e in st["entities"] if e["id"] == eid), None)
        if e is None:
            return True
        d = math.hypot(e["x"] - st["x"], e["z"] - st["z"])
        if d > reach:
            self.walk_to(e["x"], e["z"], timeout=1.5, tol=reach - 0.5)
        self.lookat(e["x"], e["y"] + 0.6, e["z"])
        time.sleep(0.08)
        self.mouse("left", "tap")
        time.sleep(0.55)
    return False


def _respawn(self):
    if self.screen() == "DeathScreen":
        self.cmd("mousepos 480 298")
        time.sleep(0.3)
        self.mouse("left", "tap")
        time.sleep(3)


C.collect, C.screen, C.open_inv, C.close, C.craft = _collect, _screen, _open_inv, _close, _craft
C.craft_table, C.craft2, C.select, C.aim, C.use_on, C.place = _craft_table, _craft2, _select, _aim, _use_on, _place
C.fight, C.respawn = _fight, _respawn


def shot(name):
    """Screenshot of the Xvfb display (the client's own virtual screen) to ~/wp58/shots/<name>.png."""
    SHOTS.mkdir(parents=True, exist_ok=True)
    out = SHOTS / f"{name}.png"
    subprocess.run(["import", "-display", ":99", "-window", "root", str(out)], check=True)
    return out
