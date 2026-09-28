"""Weather and sleeping seen by the real 26.3 client: a vanilla-terrain world (seed 12345),
rain at the spawn, snow falling and piling up in a snowy plains, then a bed at night: the
player sleeps and the night is skipped. Screenshots of the game window go to
work/weather-view-*.png; the server's time queries show the skip.

usage: python tools/weather_view.py [--port 25586] [--keep]
"""

import argparse
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402
from blocks_view import start_server  # noqa: E402

NAME = "KilnView"
# `findbiome minecraft:snowy_plains 12345`: chunk 34 40.
SNOW = (552, 648)


def send(server, *lines, pause=0.3):
    for line in lines:
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(pause)


def shot(pid, name):
    out = e2e.WORK / f"weather-view-{name}.png"
    e2e.screenshot(pid, out)
    print("screenshot:", out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25586)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    os.environ.update(KILN_GENERATOR="noise", KILN_SEED="12345")
    os.environ.setdefault("KILN_DATAPACK", str(e2e.WORK / "generated"))
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = start_server(a.port)
    pid = None
    for attempt in range(3):
        try:
            pid = e2e.launch(a.port, NAME)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 240
        while time.time() < deadline and f"{NAME} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid):
                break
            time.sleep(1)
        if e2e.alive(pid) and f"{NAME} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")
    time.sleep(5)
    # Rain at the spawn, looking level at noon.
    send(server, f"gamemode creative {NAME}", "time set 6000", "weather rain", f"execute as {NAME} at @s run tp @s ~ ~ ~ 0 0")
    time.sleep(8)
    shot(pid, "rain")
    # Snow in a snowy plains; a fast random tick speed piles it up.
    send(server, f"tp {NAME} {SNOW[0]} 120 {SNOW[1]} 0 30", "gamerule minecraft:random_tick_speed 400")
    time.sleep(25)
    shot(pid, "snow")
    send(server, "gamerule minecraft:random_tick_speed 3", "weather clear")
    # A bed at night: sleep and wake up in the morning.
    send(server,
         f"gamemode survival {NAME}",
         f"execute at {NAME} run fill ~1 ~-1 ~-1 ~3 ~-1 ~1 minecraft:stone",
         f"execute at {NAME} run fill ~1 ~ ~-1 ~3 ~2 ~1 minecraft:air",
         f"execute at {NAME} run setblock ~2 ~ ~ minecraft:red_bed[part=head,facing=east]",
         f"execute at {NAME} run setblock ~1 ~ ~ minecraft:red_bed[part=foot,facing=east]",
         "time set 18000")
    time.sleep(3)
    send(server, "time query daytime", f"execute at {NAME} run kiln use {NAME} ~2 ~ ~")
    time.sleep(2.5)
    shot(pid, "sleeping")
    time.sleep(6)
    send(server, "time query daytime")
    time.sleep(2)
    shot(pid, "morning")
    log = e2e.server_log()
    times = re.findall(r"commands\.time\.query\.absolute.*|The time is.*|time.*query.*", log)
    for l in log.splitlines():
        if "time" in l.lower() and ("query" in l.lower() or "is " in l):
            print("server:", l.strip()[-160:])
    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l[-200:])
    for l in log.splitlines():
        if re.search(r"panicked|ERROR|WARN", l):
            print("server:", l[-200:])
    _ = times
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
