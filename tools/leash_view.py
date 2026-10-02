"""Leads, leash knots, boats on leads, the wandering trader and its trader llamas seen by the real
26.3 client: start a release server, join it with the client (KilnView), then screenshot the game
window (PrintWindow only) for each scene:
  lead     a cow and a sheep on the player's lead, a boat on a lead, two sheep on a fence knot
  trader   a wandering trader with two trader llamas on leads, and the trader's trade screen is
           not opened (a click cannot be sent from here); the client must decode the entities
Reports client decode errors and server warnings at the end.

usage: python tools/leash_view.py [--port 25582] [--keep] [--scene NAME] [--no-build]
"""

import argparse
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
    """`UUID.nameUUIDFromBytes("OfflinePlayer:" + name)` as an int array."""
    h = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    h[6] = h[6] & 0x0F | 0x30
    h[8] = h[8] & 0x3F | 0x80
    n = uuid.UUID(bytes=bytes(h)).int
    ints = [(n >> s) & 0xFFFFFFFF for s in (96, 64, 32, 0)]
    return [i - (1 << 32) if i >= 1 << 31 else i for i in ints]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25582)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("--scene", action="append", help="only these scenes (lead, trader)")
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    scenes = a.scene or ["lead", "trader"]
    if not a.no_build and subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
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

    def shot(tag):
        out = e2e.WORK / f"leash-view-{tag}.png"
        e2e.screenshot(pid_, out)
        print("screenshot:", out, flush=True)

    ints = offline_uuid(name)
    player = "[I;" + ",".join(str(i) for i in ints) + "]"
    time.sleep(3)
    console("difficulty normal")
    console("gamerule minecraft:spawn_mobs false")
    console("gamerule minecraft:spawn_wandering_traders false")
    console("time set 6000")
    console(f"gamemode creative {name}")

    if "lead" in scenes:
        console(f"tp {name} 8.5 -60 18.5 180 15")
        time.sleep(1)
        # Mobs on the player's lead (a lead is its holder's UUID; they find the player on their first tick).
        console(f"summon minecraft:cow 6.5 -60 14.5 {{PersistenceRequired:1b,leash:{{UUID:{player}}}}}", 0.1)
        console(f"summon minecraft:sheep 10.5 -60 14.5 {{PersistenceRequired:1b,leash:{{UUID:{player}}}}}", 0.1)
        console(f"summon minecraft:oak_boat 8.5 -60 12.5 {{leash:{{UUID:{player}}}}}", 0.1)
        # A fence with a knot and two sheep on it.
        console("setblock 14 -60 14 minecraft:oak_fence", 0.1)
        console("setblock 14 -61 14 minecraft:stone", 0.1)
        console("summon minecraft:sheep 12.5 -60 12.5 {PersistenceRequired:1b,leash:[I;14,-60,14]}", 0.1)
        console("summon minecraft:sheep 16.5 -60 12.5 {PersistenceRequired:1b,leash:[I;14,-60,14]}", 0.1)
        time.sleep(3)
        shot("lead")
        # Walk away: the leads pull and the animals follow.
        console(f"tp {name} 8.5 -60 26.5 180 15")
        time.sleep(3)
        shot("lead-pulled")
        console("kill @e[type=!minecraft:player]", 1.0)
        console("setblock 14 -60 14 minecraft:air", 0.5)

    if "trader" in scenes:
        console(f"tp {name} 8.5 -60 18.5 180 15")
        time.sleep(1)
        trader = "[I;11,22,33,44]"
        console(f"summon minecraft:wandering_trader 8.5 -60 12.5 {{UUID:{trader},PersistenceRequired:1b,DespawnDelay:48000,wander_target:[I;8,-60,8]}}", 0.1)
        console(f"summon minecraft:trader_llama 6.5 -60 12.5 {{PersistenceRequired:1b,leash:{{UUID:{trader}}}}}", 0.1)
        console(f"summon minecraft:trader_llama 10.5 -60 12.5 {{PersistenceRequired:1b,leash:{{UUID:{trader}}}}}", 0.1)
        time.sleep(4)
        shot("trader")
        time.sleep(6)
        shot("trader-walked")
        console("kill @e[type=!minecraft:player]", 1.0)

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
