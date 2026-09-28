"""Advancements and the recipe book seen by the real 26.3 client: join a release server,
grant an advancement (its toast and chat announcement), then unlock recipes, open a crafting
table (`/kiln use`) with its recipe book open (`/kiln recipebook`); screenshots of the game window only (PrintWindow): work/advancements-view-*.png.

usage: python tools/advancements_view.py [--port 25587] [--keep]
"""

import argparse
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import blocks_view  # noqa: E402
import e2e  # noqa: E402

SCENE = """
gamemode survival KilnView
time set 6000
fill -2 -60 12 18 -50 30 air
setblock 8 -60 18 crafting_table
tp KilnView 8.5 -60 20.5 180 30
clear KilnView
give KilnView oak_planks 16
give KilnView cobblestone 16
give KilnView stick 8
"""


def send(server, lines):
    for line in lines.strip().splitlines():
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(0.2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25587)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = blocks_view.start_server(a.port)
    name = "KilnView"
    pid = None
    for attempt in range(3):
        try:
            pid = e2e.launch(a.port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{name} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid):
                break
            time.sleep(1)
        if e2e.alive(pid) and f"{name} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")
    send(server, SCENE)
    time.sleep(4)

    def shot(tag):
        out = e2e.WORK / f"advancements-view-{tag}.png"
        e2e.screenshot(pid, out)
        print("screenshot:", out)

    # A goal and a task: toasts (top right) and the chat announcements.
    send(server, "advancement revoke KilnView everything\nadvancement grant KilnView only minecraft:story/mine_stone")
    time.sleep(1.5)
    shot("toast")
    # Recipes: everything unlocked; the crafting table's book shows them.
    send(server, "recipe give KilnView *")
    time.sleep(2)
    send(server, "kiln use KilnView 8 -60 18")
    time.sleep(3)
    shot("crafting")
    # The book open (the client would send this setting when its button is clicked; posted
    # clicks land at the window center, since the cursor does not move without focus).
    send(server, "kiln recipebook KilnView")
    time.sleep(1)
    send(server, "kiln use KilnView 8 -60 18")
    time.sleep(3)
    shot("recipe-book")
    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    problems = [l for l in client_log.splitlines()
                if re.search(r"disconnect|Failed to decode|Exception|Error", l) and "Realms" not in l]
    for l in problems[-20:]:
        print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
