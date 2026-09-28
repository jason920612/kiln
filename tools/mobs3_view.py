"""Slice-3 mobs seen by the real 26.3 client: start a release server, join it with the client
(KilnView), then screenshot the game window (PrintWindow only) for each scene:
  effects  mobs with effects (particles, a glowing zombie, an invisible one, a lingering cloud)
  raid     a village raid started by raid omen (the raid bar, illagers)
  dragon   the first entry into the End: the dragon fight
Reports client decode errors and server warnings at the end.

usage: python tools/mobs3_view.py [--port 25585] [--keep] [--scene NAME]
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
    ap.add_argument("--port", type=int, default=25585)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("--scene", action="append", help="only these scenes (effects, raid, dragon)")
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    scenes = a.scene or ["effects"]
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

    shots = []

    def shot(tag):
        out = e2e.WORK / f"mobs3-view-{tag}.png"
        e2e.screenshot(pid_, out)
        shots.append(out)
        print("screenshot:", out, flush=True)

    time.sleep(3)
    console("difficulty normal")
    console("gamerule minecraft:spawn_mobs false")
    console("time set 6000")
    console(f"gamemode creative {name}")

    if "effects" in scenes:
        console(f"tp {name} 8.5 -60 18.5 180 15")
        time.sleep(1)
        kinds = ["pig", "cow", "sheep", "zombie", "skeleton", "villager"]
        for i, k in enumerate(kinds):
            console(f"summon minecraft:{k} {2.5 + i * 2.5} -60 10.5 {{NoAI:1b,PersistenceRequired:1b}}", 0.1)
        console("effect give @e[type=minecraft:pig] minecraft:poison 600 0")
        console("effect give @e[type=minecraft:cow] minecraft:speed 600 1")
        console("effect give @e[type=minecraft:sheep] minecraft:regeneration 600 0")
        console("effect give @e[type=minecraft:zombie] minecraft:glowing 600 0")
        console("effect give @e[type=minecraft:skeleton] minecraft:invisibility 600 0")
        console("effect give @e[type=minecraft:villager] minecraft:weakness 600 0")
        time.sleep(3)
        shot("effects")
        # A lingering potion's cloud: a lingering potion item thrown down by a dispenser is not
        # available from the console; summon the cloud with its potion instead.
        console('summon minecraft:area_effect_cloud 8.5 -60 14.5 {Radius:2.5f,Duration:600,WaitTime:0,potion_contents:{potion:"minecraft:poison"}}')
        time.sleep(2)
        shot("effects-cloud")
        console("effect clear @e[type=minecraft:skeleton]")
        time.sleep(1.5)
        shot("effects-cleared")
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
