"""Block behaviour seen by the real 26.3 client: start a release server, join it with the
client, build a scene from the console (redstone line lit by a lever, a sticky piston pushed
by a redstone block, flowing water, connected fences, falling sand, a TNT crater), then take a screenshot
of the game window (work/blocks-view.png) and report client decode errors.

usage: python tools/blocks_view.py [--port 25570] [--keep]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402

# Superflat: grass at y -61, the scene stands on it. The client looks north from z 22.
SCENE = """
gamemode creative KilnView
time set 6000
fill -2 -60 0 18 -50 16 air
fill 3 -60 12 10 -60 12 redstone_wire
setblock 11 -60 12 redstone_lamp
setblock 2 -60 12 lever[face=floor,facing=east]
setblock 2 -60 12 lever[face=floor,facing=east,powered=true]
setblock 6 -60 8 sticky_piston[facing=east]
setblock 7 -60 8 stone
setblock 5 -60 8 redstone_block
setblock 14 -60 4 water
fill 0 -60 2 6 -60 2 oak_fence
fill 12 -55 10 12 -52 10 sand
setblock 16 -61 14 tnt
setblock 17 -61 14 redstone_block
tp KilnView 8 -58 22 180 25
execute if block 11 -60 12 minecraft:redstone_lamp[lit=true]
execute if block 3 -60 12 minecraft:redstone_wire[power=15]
"""


def start_server(port):
    e2e.stop_server(port)
    env = dict(os.environ, KILN_PORT=str(port), RUST_LOG="info", KILN_OPS="KilnView")
    exe = e2e.WORK / f"kiln-{port}{e2e.EXE.suffix}"
    shutil.copy2(e2e.EXE, exe)
    log = open(e2e.SERVER_LOG, "w", encoding="utf-8")
    flags = 0x00000200 | 0x01000000 if os.name == "nt" else 0
    p = subprocess.Popen([str(exe)], cwd=e2e.ROOT, env=env, stdin=subprocess.PIPE, stdout=log,
                         stderr=subprocess.STDOUT, creationflags=flags, text=True)
    (e2e.WORK / f"server-{port}.pid").write_text(str(p.pid))
    for _ in range(100):
        if "listening on" in e2e.server_log():
            return p
        time.sleep(0.1)
    sys.exit("server did not start:\n" + e2e.server_log())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25570)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = start_server(a.port)
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
    for line in SCENE.strip().splitlines():
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(0.2)
    time.sleep(6)
    shot = e2e.WORK / "blocks-view.png"
    e2e.screenshot(pid, shot)
    print("screenshot:", shot)
    # Close up of the lamp end of the redstone line.
    server.stdin.write("tp KilnView 11.5 -59 15.5 180 30\n")
    server.stdin.flush()
    time.sleep(3)
    close = e2e.WORK / "blocks-view-close.png"
    e2e.screenshot(pid, close)
    print("screenshot:", close)
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
