"""Data pack functions seen by the real 26.3 client: start a release server with the
tools/datapacks/kilnview pack (KILN_DATAPACKS), join it with the client, and take screenshots
of the game window: #minecraft:load made a boss bar that #minecraft:tick fills while it
shows a counter in the action bar (work/functions-view-tick.png); then a macro function run
from the console builds a wall in front of the player and titles it
(work/functions-view.png).

usage: python tools/functions_view.py [--port 25583] [--keep]
"""

import argparse
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402
import lobby_view  # noqa: E402

PACK = e2e.ROOT / "tools" / "datapacks" / "kilnview"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25583)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    packs = e2e.WORK / "functions-view-packs"
    shutil.rmtree(packs, ignore_errors=True)
    shutil.copytree(PACK, packs / PACK.name)
    os.environ["KILN_DATAPACKS"] = str(packs)
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = lobby_view.start_server(a.port, None)
    name = "KilnView"
    pid = lobby_view.join_client(a.port, name)
    if pid is None:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")
    lobby_view.run_scene(server, "gamemode creative KilnView\ntime set 6000\ntp KilnView ~ ~ ~ 180 10")
    time.sleep(6)
    shot = e2e.WORK / "functions-view-tick.png"
    e2e.screenshot(pid, shot)
    print("screenshot:", shot)
    scene = """
execute as KilnView at @s run function kilnview:build {block:"gold_block",top:"sea_lantern"}
"""
    lobby_view.run_scene(server, scene)
    time.sleep(4)
    shot = e2e.WORK / "functions-view.png"
    e2e.screenshot(pid, shot)
    print("screenshot:", shot)
    lobby_view.report()
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
