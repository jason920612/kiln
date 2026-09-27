"""Mob effects, fire and air seen by the real 26.3 client: start a release server, join it with
the client (KilnView, survival), give it effects with /effect, set it on fire in a fire block,
then put its head under water: screenshots of the game window with the effect icons, the
burning overlay and the air bubbles, then report client decode errors.

usage: python tools/effects_view.py [--port 25586] [--keep]
"""

import argparse
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
sys.dont_write_bytecode = True
import blocks_view  # noqa: E402
import e2e  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25586)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    # The vanilla datapack (loot, recipes) from the work directory.
    os.environ.setdefault("KILN_DATAPACK", str(e2e.WORK / "generated"))
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = blocks_view.start_server(a.port)

    def console(cmd):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(0.3)

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

    time.sleep(3)
    console("time set 6000")
    console("gamerule minecraft:natural_health_regeneration false")
    console(f"gamemode survival {name}")
    console(f"tp {name} 8.5 -60 12.5 180 30")
    time.sleep(2)
    for effect in ("speed 120 1", "regeneration 120 0", "strength 120 2", "night_vision 120 0",
                   "poison 8 0", "haste infinite 1", "jump_boost 120 1 true"):
        console(f"effect give {name} minecraft:{effect}")
    time.sleep(2.5)
    icons = e2e.WORK / "effects-view-icons.png"
    e2e.screenshot(pid_, icons)
    print("screenshot:", icons)

    # Standing in fire: the counter climbs to 0, then 8 seconds of burning.
    console("setblock 8 -60 12 minecraft:fire")
    time.sleep(3.0)
    burning = e2e.WORK / "effects-view-burning.png"
    e2e.screenshot(pid_, burning)
    print("screenshot:", burning)

    # Head under water: the fire goes out and the air bubbles go down.
    console("setblock 8 -60 12 minecraft:water")
    console("setblock 8 -59 12 minecraft:water")
    console("effect clear KilnView minecraft:poison")
    time.sleep(6.0)
    air = e2e.WORK / "effects-view-air.png"
    e2e.screenshot(pid_, air)
    print("screenshot:", air)

    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN|died|effect", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid_), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
