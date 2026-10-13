"""Python side of the wp58 client driver (tools/wp58/ClientCtl.java, loaded as a java agent).

Usable as a module (`from ctl import C; c = C()`) or a CLI: python tools/wp58/ctl.py state
"""
import json
import math
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

# Runs on the VM next to the server (~/wp58) and the real client (Xvfb :99, ~/mc-client).
HOME = Path.home()
PLAY = HOME / os.environ.get("KILN_PLAY_DIR", "wp58")  # wp59 sets KILN_PLAY_DIR=wp59
SHOTS = PLAY / "shots"


def server(cmd):
    """Run a console command on the wp58 server (its console is the fifo $KILN_PLAY_DIR/console)."""
    with open(PLAY / "console", "w") as f:
        f.write(cmd + chr(10))


def server_log(n=40):
    """The last n server log lines without the periodic mspt / chunk totals reports."""
    lines = (PLAY / "server.log").read_text(errors="replace").splitlines()
    return chr(10).join(l[:240] for l in [l for l in lines if "chunk totals" not in l and "mspt mean" not in l][-n:])


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


TIERS = ["netherite", "diamond", "iron", "stone", "golden", "wooden"]


def _tool_kind(block):
    b = block.split("[")[0].replace("Block{minecraft:", "").rstrip("}")
    if any(k in b for k in ("log", "planks", "wood", "stem", "hyphae", "chest", "table", "bookshelf", "fence", "door", "barrel", "ladder")) and "nether_brick" not in b:
        return "axe"
    if any(k in b for k in ("dirt", "sand", "gravel", "grass_block", "mud", "clay", "snow", "soul_s", "podzol", "mycelium", "farmland", "path")):
        return "shovel"
    if any(k in b for k in ("stone", "ore", "cobble", "deepslate", "netherrack", "obsidian", "brick", "furnace", "basalt", "blackstone", "terracotta", "andesite", "diorite", "granite", "tuff", "calcite", "bone_block", "iron_block", "anvil", "prismarine", "purpur", "end_stone")):
        return "pickaxe"
    return None


def _equip_for(self, block):
    kind = _tool_kind(block)
    inv = self.state(0)["inv"]
    if kind is None:
        return
    for tier in TIERS:
        want = f"minecraft:{tier}_{kind}"
        for e in inv:
            if e["id"] == want:
                if e["slot"] < 9:
                    if self.state(0)["selected"] != e["slot"]:
                        self.hotbar(e["slot"] + 1)
                        time.sleep(0.1)
                else:
                    self.select(want)
                return


def _mine(self, x, y, z, timeout=20, stand=True):
    """Look at the centre of a block and hold attack until it is gone."""
    t0 = time.time()
    self.equip_for(self.block(x, y, z))
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
C.equip_for = _equip_for
C.near = _near


def _ground(self, x, z, y0=200):
    """Y of the first free block above the top solid block of column (x, z) as loaded by the client."""
    for y in range(y0, -64, -1):
        b = self.block(x, y, z)
        if "air" not in b.split("[")[0] and "leaves" not in b and "grass}" not in b.split("[")[0].replace("short_", "").replace("tall_", "") + "}" and "fern" not in b and "flower" not in b:
            return y + 1
    return None


def _tp(self, x, y, z):
    server(f"tp KilnTest {x} {y} {z}")
    time.sleep(2.0)


C.ground, C.tp = _ground, _tp


def _dig(self, sub, count, radius=16, exact=None, timeout=60):
    """Mine `count` blocks whose id contains `sub` (nearest first), picking up the drops."""
    t0 = time.time()
    done = 0
    while done < count and time.time() - t0 < timeout:
        hits = [h for h in self.find(sub, radius, 60) if exact is None or self.block(*h).split("[")[0] == "Block{" + exact + "}"]
        if not hits:
            return done
        h = hits[0]
        self.near(*h, dist=3.0)
        if self.mine(*h, timeout=15):
            done += 1
            self.collect(5, 4)
    return done


C.dig = _dig


def _shaft_down(self, x, z, y_to, timeout=240):
    """Stand over column (x, z) and dig straight down to level y_to (inclusive), collecting drops."""
    t0 = time.time()
    st = self.state(0)
    self.walk_to(x + 0.5, z + 0.5, timeout=8, tol=0.3)
    y = math.floor(self.state(0)["y"]) - 1
    while y >= y_to and time.time() - t0 < timeout:
        b = self.block(x, y, z)
        if "air" not in b.split("[")[0] and "water" not in b and "lava" not in b:
            self.mine(x, y, z, timeout=15)
        elif "lava" in b:
            raise RuntimeError(f"lava at {x},{y},{z}")
        self.collect(3, 3)
        y -= 1
        time.sleep(0.1)
    return self.state(0)["y"]


C.shaft_down = _shaft_down


FACES = {"up": (0, 1, 0), "down": (0, -1, 0), "north": (0, 0, -1), "south": (0, 0, 1), "east": (1, 0, 0), "west": (-1, 0, 0)}


def _solid(b):
    n = b.split("[")[0]
    return not any(k in n for k in ("air", "water", "lava", "grass}", "torch", "fern", "flower", "litter", "vine", "snow}", "rail"))


def _place_near(self, item, avoid_own_cell=True):
    """Place `item` on any free spot touching a solid block near the player. Returns the new block's position."""
    st = self.state(0)
    px, py, pz = math.floor(st["x"]), math.floor(st["y"]), math.floor(st["z"])
    cands = []
    for dx in range(-2, 3):
        for dy in range(-2, 2):
            for dz in range(-2, 3):
                t = (px + dx, py + dy, pz + dz)
                if not _solid(self.block(*t)):
                    continue
                for face, (fx, fy, fz) in FACES.items():
                    c = (t[0] + fx, t[1] + fy, t[2] + fz)
                    if avoid_own_cell and c[0] == px and c[2] == pz and c[1] in (py, py + 1):
                        continue
                    if "air" in self.block(*c).split("[")[0] and "air" in self.block(c[0], c[1] + 1, c[2]).split("[")[0] or face != "up" and "air" in self.block(*c).split("[")[0]:
                        d = (t[0] + 0.5 - st["x"]) ** 2 + (t[1] + 0.5 - st["y"] - 1.6) ** 2 + (t[2] + 0.5 - st["z"]) ** 2
                        cands.append((d, t, face, c))
    cands.sort()
    self.select(item)
    off = {"up": (0.5, 0.95, 0.5), "north": (0.5, 0.5, 0.05), "south": (0.5, 0.5, 0.95),
           "east": (0.95, 0.5, 0.5), "west": (0.05, 0.5, 0.5), "down": (0.5, 0.05, 0.5)}
    for d, t, face, cpos in cands[:12]:
        h = self.aim(*t, *off[face])
        if h and h.get("type") == "block" and (h["x"], h["y"], h["z"]) == t and h["face"] == face:
            self.mouse("right", "tap")
            time.sleep(0.3)
            if not "air" in self.block(*cpos).split("[")[0]:
                return cpos
    return None


C.place_near = _place_near


def _tunnel_to(self, h, maxsteps=40):
    """Dig and walk towards block h (x, y, z) until it is mined. Returns True when mined."""
    for _ in range(maxsteps):
        b = self.block(*h)
        if "air" in b.split("[")[0]:
            return True
        st = self.state(0)
        close = abs(st["x"] - (h[0] + .5)) < 3.8 and abs(st["z"] - (h[2] + .5)) < 3.8 and abs(st["y"] + 1.6 - (h[1] + .5)) < 3.8
        aim = self.aim(*h)
        if close and aim and aim.get("type") == "block" and (aim["x"], aim["y"], aim["z"]) == tuple(h):
            self.mine(*h, timeout=20)
            self.collect(5, 5)
            return True
        if aim and aim.get("type") == "block":
            self.mine(aim["x"], aim["y"], aim["z"], timeout=15)
            self.collect(4, 3)
        self.hold("w", 0.3)
    return False


C.tunnel_to = _tunnel_to


def _safe_tp(self, x, z, y_hint=100):
    """Teleport above column (x, z) under slow falling, wait for its chunk, settle on the ground."""
    server("effect give KilnTest minecraft:slow_falling 60 0 true")
    self.tp(x + 0.5, y_hint + 20, z + 0.5)
    for _ in range(40):
        g = self.ground(math.floor(x), math.floor(z), y_hint + 40)
        if g is not None and self.block(math.floor(x), g - 1, math.floor(z)) != "":
            break
        time.sleep(0.5)
    g = self.ground(math.floor(x), math.floor(z), y_hint + 40)
    if g is not None:
        server(f"tp KilnTest {x + 0.5} {g} {z + 0.5}")
        time.sleep(1.5)
    server("effect clear KilnTest minecraft:slow_falling")
    return g


C.safe_tp = _safe_tp


def _place_at(self, item, tgt, tries=3):
    """Put `item` into the free cell tgt by clicking a solid neighbour's face that touches it
    (sneaking, so containers and workbenches are not opened by the click)."""
    off = {"up": (0.5, 0.95, 0.5), "north": (0.5, 0.5, 0.05), "south": (0.5, 0.5, 0.95),
           "east": (0.95, 0.5, 0.5), "west": (0.05, 0.5, 0.5), "down": (0.5, 0.05, 0.5)}
    opposite = {"up": "down", "down": "up", "north": "south", "south": "north", "east": "west", "west": "east"}
    self.select(item)
    self.key("shift", "down")
    time.sleep(0.2)
    try:
        for _ in range(tries):
            if item.split(":")[1] in self.block(*tgt):
                return True
            for face, (fx, fy, fz) in FACES.items():
                sup = (tgt[0] + fx, tgt[1] + fy, tgt[2] + fz)
                if not _solid(self.block(*sup)):
                    continue
                click_face = opposite[face]  # the support's face that touches the target cell
                h = self.aim(*sup, *off[click_face])
                if h and h.get("type") == "block" and (h["x"], h["y"], h["z"]) == sup and h["face"] == click_face:
                    self.mouse("right", "tap")
                    time.sleep(0.3)
                    if item.split(":")[1] in self.block(*tgt):
                        return True
    finally:
        self.key("shift", "up")
        time.sleep(0.1)
    return False


C.place_at = _place_at


def _pillar(self, n, item="minecraft:dirt"):
    """Jump and place `item` under the feet n times (the player climbs n blocks)."""
    self.select(item)
    y0 = self.state(0)["y"]
    for i in range(n):
        target = y0 + i + 1
        for _ in range(8):
            self.look(0, 90)
            self.key("space", "down")
            time.sleep(0.28)
            self.mouse("right", "tap")
            self.key("space", "up")
            time.sleep(0.45)
            if self.state(0)["y"] >= target - 0.05:
                break
    return self.state(0)["y"]


C.pillar = _pillar


def _wait_ready(self, timeout=30):
    """Wait until the client is in the game with no loading screen or overlay."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        s = self.state(0)
        if s["inGame"] and s["overlay"] is None and s["screen"] in (None,):
            return True
        if s["screen"] == "DeathScreen":
            self.respawn()
        time.sleep(0.5)
    return False


C.wait_ready = _wait_ready


def _clear_plants(self, x0, y0, z0, x1, y1, z1):
    """Break the plants (non-solid, non-air blocks) inside the box so they cannot intercept aiming."""
    n = 0
    for z in range(z0, z1 + 1):
        for x in range(x0, x1 + 1):
            for y in range(y0, y1 + 1):
                b = self.block(x, y, z)
                if "air" not in b.split("[")[0] and not _solid(b) and "water" not in b and "lava" not in b and "portal" not in b:
                    self.near(x, y, z, dist=3.5)
                    self.mine(x, y, z, timeout=5)
                    n += 1
    return n


C.clear_plants = _clear_plants


def _workbench(self):
    """Position of a crafting table within reach, placing one from the inventory if none is near."""
    st = self.state(0)
    px, py, pz = math.floor(st["x"]), math.floor(st["y"]), math.floor(st["z"])
    for dx in range(-3, 4):
        for dy in range(-2, 3):
            for dz in range(-3, 4):
                if "crafting_table" in self.block(px + dx, py + dy, pz + dz):
                    return (px + dx, py + dy, pz + dz)
    self.clear_plants(px - 2, py - 1, pz - 2, px + 2, py + 2, pz + 2)
    if "minecraft:crafting_table" not in self.inv():
        self.open_inv()
        self.craft2(["PP", "PP"], {"P": "minecraft:oak_planks"})
    return self.place_near("minecraft:crafting_table")


C.workbench = _workbench


def _findbox(self, sub, x0, y0, z0, x1, y1, z1, n=40):
    return json.loads(self.cmd(f"findbox {sub} {x0} {y0} {z0} {x1} {y1} {z1} {n}"))


C.findbox = _findbox


def _shoot(self, x, y, z, charge=1.2, lead=(0, 0, 0)):
    """Draw the bow in hand at the point (x, y, z) (+ lead) for `charge` seconds and release."""
    self.lookat(x + lead[0], y + lead[1], z + lead[2])
    self.mouse("right", "down")
    time.sleep(charge)
    self.lookat(x + lead[0], y + lead[1], z + lead[2])
    self.mouse("right", "up")


C.shoot = _shoot


def solve_pitch(h, dy, v=3.0, drag=0.99, g=0.05):
    """Launch pitch (degrees, negative = up) at which an arrow of speed v reaches horizontal distance h
    at height difference dy (`dy` up from the eye): the lower of the two solutions, by simulation."""
    best = None
    for k in range(-890, 891):
        pitch = k / 10.0
        vy = -math.sin(math.radians(pitch)) * v
        vh = math.cos(math.radians(pitch)) * v
        x = y = 0.0
        err = None
        for _ in range(400):
            x += vh
            y += vy
            vh *= drag
            vy = vy * drag - g
            if x >= h:
                err = abs(y - dy)
                break
        if err is not None and (best is None or err < best[0]):
            best = (err, pitch)
    return best[1] if best else None


def _shoot_ballistic(self, x, y, z, charge=1.25):
    """Aim a fully drawn bow at the point so that the arrow's drop is accounted for, and loose."""
    st = self.state(0)
    ex, ey, ez = st["x"], st["y"] + 1.62, st["z"]
    dx, dz = x - ex, z - ez
    h = math.hypot(dx, dz)
    pitch = solve_pitch(h, y - ey)
    yaw = math.degrees(math.atan2(-dx, dz))
    self.look(yaw, pitch if pitch is not None else 0)
    self.mouse("right", "down")
    time.sleep(charge)
    self.look(yaw, pitch if pitch is not None else 0)
    self.mouse("right", "up")


C.shoot_ballistic = _shoot_ballistic


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
    for _ in range(5):
        if self.screen() == "InventoryScreen":
            break
        if self.screen() is not None:
            self.close()
        self.key("e")
        time.sleep(0.5)
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
    """Look at a point of block (x, y, z); the hit result is refreshed once per rendered frame
    (software GL is slow), so wait until it names the block. A sneaking player's eye is lower
    than the server-side value the agent uses, so on a miss the pitch is nudged downwards."""
    self.lookat(x + fx, y + fy, z + fz)
    time.sleep(wait)
    for nudge in (0, 0, 2, 4, 6, -2):
        for _ in range(4):
            h = self.state(0).get("hit")
            if h and h.get("type") == "block" and (h["x"], h["y"], h["z"]) == (x, y, z):
                return h
            time.sleep(0.15)
        if nudge:
            st = self.state(0)
            self.look(st["yaw"], max(-90, min(90, st["pitch"] + nudge)))
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
