"""Slice-2 mobs seen by the real 26.3 client: start a release server, join it with the client
(KilnView), then screenshot the game window (PrintWindow only) for: an enderman teleporting
out of water, a tamed wolf following its owner, a villager's trade screen (the client
right-clicks through a posted mouse message), and a row of the new mob types.
Reports client decode errors and server warnings at the end.

usage: python tools/mobs2_view.py [--port 25585] [--keep] [--scene NAME]
"""

import argparse
import ctypes
import hashlib
import os
import re
import subprocess
import sys
import time
import uuid

sys.path.insert(0, os.path.dirname(__file__))
sys.dont_write_bytecode = True
import blocks_view  # noqa: E402
import e2e  # noqa: E402


def offline_uuid(name):
    """`UUIDUtil.createOfflinePlayerUUID`: name-based (MD5) UUID of "OfflinePlayer:<name>"."""
    return uuid.UUID(bytes=hashlib.md5(f"OfflinePlayer:{name}".encode()).digest(), version=3)


def int_array(u):
    v = u.int
    parts = [(v >> s) & 0xFFFFFFFF for s in (96, 64, 32, 0)]
    return "[I;" + ",".join(str(p - (1 << 32) if p >= 1 << 31 else p) for p in parts) + "]"


def right_click(pid):
    """Posts a right mouse button click to the game window's center (no focus stealing)."""
    user32 = ctypes.windll.user32
    hwnd = None

    @ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)
    def cb(h, _):
        nonlocal hwnd
        p = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(h):
            hwnd = h
            return False
        return True

    user32.EnumWindows(cb, 0)
    if not hwnd:
        return False
    class RECT(ctypes.Structure):
        _fields_ = [("l", ctypes.c_long), ("t", ctypes.c_long), ("r", ctypes.c_long), ("b", ctypes.c_long)]
    r = RECT()
    user32.GetClientRect(hwnd, ctypes.byref(r))
    lp = ((r.b // 2) << 16) | (r.r // 2)
    user32.PostMessageW(hwnd, 0x0204, 0x0002, lp)  # WM_RBUTTONDOWN
    time.sleep(0.08)
    user32.PostMessageW(hwnd, 0x0205, 0, lp)  # WM_RBUTTONUP
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25585)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("--scene", action="append", help="only these scenes (enderman, wolf, villager, row)")
    a = ap.parse_args()
    scenes = set(a.scene or ["enderman", "wolf", "villager", "row"])
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = blocks_view.start_server(a.port)

    def console(cmd, wait=0.3):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(wait)

    name = "KilnView"
    pid_ = None
    for attempt in range(3):
        try:
            pid_ = e2e.launch(a.port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{name} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid_):
                break
            time.sleep(1)
        if e2e.alive(pid_) and f"{name} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")

    shots = []

    def shot(tag):
        out = e2e.WORK / f"mobs2-view-{tag}.png"
        e2e.screenshot(pid_, out)
        shots.append(out)
        print("screenshot:", out, flush=True)

    time.sleep(3)
    console("difficulty normal")
    console("gamerule minecraft:spawn_mobs false")
    console("gamerule minecraft:natural_health_regeneration false")
    console("time set 6000")
    console(f"gamemode creative {name}")
    console(f"tp {name} 8.5 -60 14.5 180 15")
    time.sleep(2)

    if "enderman" in scenes:
        # An enderman standing in water teleports away (EndermanHurtByWater / isSensitiveToWater).
        console("summon minecraft:enderman 8.5 -60 7.5")
        time.sleep(2)
        shot("enderman-before")
        console("setblock 8 -60 7 minecraft:water")
        time.sleep(1.5)
        shot("enderman-after")
        log = e2e.server_log()
        console("kill @e[type=minecraft:enderman]", 1.0)
        console("setblock 8 -60 7 minecraft:air")
        console("fill 0 -60 0 16 -58 16 minecraft:air replace minecraft:water", 0.5)

    if "wolf" in scenes:
        # A wolf tamed by KilnView follows it when it walks off (FollowOwnerGoal).
        owner = int_array(offline_uuid(name))
        console(f"summon minecraft:wolf 8.5 -60 10.5 {{Owner:{owner},PersistenceRequired:1b}}")
        time.sleep(2)
        shot("wolf-tamed")
        console(f"tp {name} 20.5 -60 20.5 180 15")
        time.sleep(4)
        shot("wolf-following")
        console("kill @e[type=minecraft:wolf]", 1.0)
        console(f"tp {name} 8.5 -60 14.5 180 15")
        time.sleep(1)

    if "villager" in scenes:
        # A farmer with trades; the client right-clicks it and the trade screen opens.
        console(f"tp {name} 8.5 -60 11.5 180 10")
        console("summon minecraft:villager 8.5 -60 9.5 {NoAI:1b,VillagerData:{profession:\"minecraft:farmer\",level:2,type:\"minecraft:plains\"}}")
        time.sleep(2)
        right_click(pid_)
        time.sleep(1.5)
        shot("villager-trade")
        console("kill @e[type=minecraft:villager]", 1.0)

    if "row" in scenes:
        console(f"tp {name} 8.5 -60 20.5 180 10")
        kinds = ["husk", "stray", "drowned", "zombified_piglin", "wither_skeleton", "witch", "slime", "magma_cube",
                 "blaze", "wolf", "cat", "horse", "donkey", "iron_golem", "villager", "piglin", "hoglin", "strider"]
        for i, k in enumerate(kinds):
            console(f"summon minecraft:{k} {-4.5 + (i % 9) * 3} -60 {8.5 + (i // 9) * 4} {{NoAI:1b,PersistenceRequired:1b}}", 0.1)
        time.sleep(3)
        shot("row")

    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid_), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
