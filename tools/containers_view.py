"""Container blocks seen by the real 26.3 client: start a release server, join it with the
client, build a scene from the console (a named chest with items, a furnace smelting iron, a
hopper chain feeding a chest read by a comparator lighting a lamp), then open the menus as if
the player clicked the blocks (`/kiln use`) and take screenshots of the game window:
work/containers-view-chest.png, -furnace.png, -hoppers.png and -hopper-chest.png. Reports client
decode errors.

usage: python tools/containers_view.py [--port 25589] [--keep]
Set KILN_DATAPACK to the vanilla datapack (work/generated) when running from a worktree.
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

# Superflat: grass at y -61. The player stands at z 22 looking north.
SCENE = """
gamemode creative KilnView
time set 6000
fill -2 -60 10 14 -50 20 air
setblock 8 -60 19 chest[facing=south]{CustomName:"Kiln Chest",Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:4b,id:"minecraft:oak_log",count:64},{Slot:13b,id:"minecraft:ender_pearl",count:16},{Slot:22b,id:"minecraft:iron_sword",count:1}]}
setblock 10 -60 19 furnace[facing=south]{Items:[{Slot:0b,id:"minecraft:raw_iron",count:8},{Slot:1b,id:"minecraft:coal",count:4}]}
setblock 4 -57 16 chest{Items:[{Slot:0b,id:"minecraft:cobblestone",count:20},{Slot:1b,id:"minecraft:gold_ingot",count:5}]}
setblock 4 -58 16 hopper[facing=down]
setblock 4 -59 16 hopper[facing=east]
setblock 5 -59 16 hopper[facing=down]
setblock 5 -60 16 chest[facing=south]
setblock 6 -60 16 comparator[facing=west]
setblock 7 -60 16 redstone_lamp
tp KilnView 8.5 -60 22 180 20
"""


def send(server, lines):
    for line in lines.strip().splitlines():
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(0.2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25589)
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
    shots = []

    def shot(tag):
        out = e2e.WORK / f"containers-view-{tag}.png"
        e2e.screenshot(pid, out)
        shots.append(out)
        print("screenshot:", out)

    # The chest, as if clicked.
    send(server, "kiln use KilnView 8 -60 19")
    time.sleep(3)
    shot("chest")
    # The furnace (opening it closes the chest), smelting for a while already.
    send(server, "kiln use KilnView 10 -60 19")
    time.sleep(4)
    shot("furnace")
    # Walking away closes the menu (out of reach); then look at the hopper chain.
    send(server, "tp KilnView 40 -60 40")
    time.sleep(2)
    send(server, "tp KilnView 5.5 -60 21 200 25")
    time.sleep(6)
    shot("hoppers")
    send(server, "tp KilnView 5.5 -60 19 180 30\nkiln use KilnView 5 -60 16")
    time.sleep(3)
    shot("hopper-chest")
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
