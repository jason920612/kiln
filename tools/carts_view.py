"""Cargo minecarts seen by the real 26.3 client: start a release server, join it with the client,
lay rails and put a chest, a hopper, a furnace (with fuel) and a TNT minecart on them, open the
chest minecart's menu as if the player clicked it (`/kiln interact`), send a TNT minecart over a
powered activator rail (it flashes, then blows up) and take screenshots of the game window:
work/carts-view-row.png, -chest-menu.png, -hopper-menu.png, -tnt-primed.png and -tnt-after.png.

usage: python tools/carts_view.py [--port 25584] [--keep]
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

# Superflat: grass at y -61. The player stands at z 22 looking north; the rails run east-west.
SCENE = """
gamemode creative KilnView
time set 6000
fill -2 -60 8 20 -50 24 air
fill 2 -60 17 14 -60 17 rail[shape=east_west]
summon chest_minecart 3.5 -59.9375 17.5 {Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:4b,id:"minecraft:oak_log",count:64},{Slot:13b,id:"minecraft:ender_pearl",count:16},{Slot:26b,id:"minecraft:iron_sword",count:1}],CustomName:"Kiln Cargo"}
summon hopper_minecart 6.5 -59.9375 17.5 {Items:[{Slot:0b,id:"minecraft:coal",count:9},{Slot:2b,id:"minecraft:apple",count:3}]}
summon furnace_minecart 9.5 -59.9375 17.5 {Fuel:2400s}
summon tnt_minecart 11.5 -59.9375 17.5 {}
summon minecart 13.5 -59.9375 17.5 {}
fill 2 -60 12 16 -60 12 rail[shape=east_west]
setblock 9 -61 12 redstone_block
setblock 9 -60 12 activator_rail[shape=east_west]
tp KilnView 8.5 -60 22 180 20
"""


def send(server, lines):
    for line in lines.strip().splitlines():
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(0.2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25584)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("--skip-build", action="store_true")
    a = ap.parse_args()
    if not a.skip_build and subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
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
        out = e2e.WORK / f"carts-view-{tag}.png"
        e2e.screenshot(pid, out)
        print("screenshot:", out)

    shot("row")
    # The chest minecart, as if clicked: a 3-row menu with its cargo.
    send(server, "kiln interact KilnView 3 -60 17")
    time.sleep(3)
    shot("chest-menu")
    # The hopper minecart (opening it closes the chest menu).
    send(server, "kiln interact KilnView 6 -60 17")
    time.sleep(3)
    shot("hopper-menu")
    send(server, "tp KilnView 40 -60 40")
    time.sleep(2)
    # A TNT minecart runs in over the powered activator rail: flashing, then the blast.
    send(server, "tp KilnView 9.5 -60 16 180 10")
    send(server, "summon tnt_minecart 3.5 -59.9375 12.5 {Motion:[0.35d,0.0d,0.0d]}")
    time.sleep(1.9)
    shot("tnt-primed")
    time.sleep(0.5)
    shot("tnt-primed-2")
    time.sleep(3.6)
    shot("tnt-after")
    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    problems = [l for l in client_log.splitlines()
                if re.search(r"disconnect|Failed to decode|Exception|Error", l) and "Realms" not in l]
    for l in problems[-20:]:
        print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN|commands.execute", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
