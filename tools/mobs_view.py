"""Mobs seen by the real 26.3 client: start a release server, join it with the client
(KilnView), then screenshot the game window (PrintWindow only) for three scenes:
passive mobs wandering, a zombie attacking the survival player, a creeper exploding.
Reports client decode errors and server warnings at the end.

usage: python tools/mobs_view.py [--port 25588] [--keep]
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
    ap.add_argument("--port", type=int, default=25588)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
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
        out = e2e.WORK / f"mobs-view-{tag}.png"
        e2e.screenshot(pid_, out)
        shots.append(out)
        print("screenshot:", out, flush=True)

    time.sleep(3)
    console("difficulty normal")
    console("gamerule minecraft:spawn_mobs false")
    console("gamerule minecraft:natural_health_regeneration false")
    console("time set 6000")
    console(f"gamemode creative {name}")
    # KilnView looks north over the flat world.
    console(f"tp {name} 8.5 -60 14.5 180 15")
    time.sleep(2)

    # Scene 1: passive mobs wander.
    for i, kind in enumerate(["pig", "cow", "sheep", "chicken", "pig", "sheep"]):
        console(f"summon minecraft:{kind} {5.5 + i * 1.5} -60 {6.5 + (i % 2) * 2}")
    time.sleep(4)
    shot("passive-1")
    time.sleep(6)
    shot("passive-2")
    console("kill @e[type=!minecraft:player]", 1.0)

    # Scene 2: a zombie walks up and hits the survival player (night: no burning).
    console("time set 14000")
    console(f"gamemode survival {name}")
    console(f"tp {name} 8.5 -60 14.5 180 10")
    console("summon minecraft:zombie 8.5 -60 4.5")
    time.sleep(3)
    shot("zombie-approach")
    time.sleep(4)
    shot("zombie-attack")
    console("kill @e[type=minecraft:zombie]", 1.0)
    console(f"effect give {name} minecraft:instant_health 1 5")
    console(f"effect give {name} minecraft:resistance 60 4")

    # Scene 3: a creeper walks up, swells and explodes.
    console("time set 6000")
    console(f"tp {name} 8.5 -60 14.5 180 20")
    console("summon minecraft:creeper 8.5 -60 6.5")
    t0 = time.time()
    swelled = False
    while time.time() - t0 < 12:
        time.sleep(0.25)
        log = e2e.server_log()
        if not swelled and time.time() - t0 > 2.2:
            shot("creeper-close")
            swelled = True
        if swelled and time.time() - t0 > 6:
            break
    shot("creeper-after")

    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN|died|Summoned|Killed", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid_), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
