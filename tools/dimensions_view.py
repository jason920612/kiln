"""The Nether and the End seen by the real 26.3 client: start a release server with vanilla
generation, join it with the client (KilnView), light a nether portal near the spawn with fire
and step in (screenshot in the Nether, next to the portal Kiln built there), then step into
an end portal (screenshot on the End's obsidian platform, looking at the island), then report
client decode errors.

usage: python tools/dimensions_view.py [--port 25587] [--seed 12345] [--keep]
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
    ap.add_argument("--port", type=int, default=25587)
    ap.add_argument("--seed", default="12345")
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    os.environ.setdefault("KILN_DATAPACK", str(e2e.WORK / "generated"))
    os.environ["KILN_GENERATOR"] = "noise"
    os.environ["KILN_SEED"] = a.seed
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = blocks_view.start_server(a.port)
    # The simulation sets up generation for three levels and loads the datapack before its
    # first tick; a client joining earlier times out.
    deadline = time.time() + 300
    while time.time() < deadline and "data packs" not in e2e.server_log():
        time.sleep(0.5)

    def console(cmd, wait=0.3):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(wait)

    name = "KilnView"
    pid = None
    for attempt in range(3):
        try:
            pid = e2e.launch(a.port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 240
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

    time.sleep(5)
    console("time set 6000")
    console(f"gamemode creative {name}")
    # A frame on a floating obsidian floor beside the player, lit with fire.
    spawn = re.search(r"world spawn \[(-?\d+), (-?\d+), (-?\d+)\]", e2e.server_log())
    sx, sy, sz = (int(v) for v in spawn.groups()) if spawn else (0, 80, 0)
    y = sy + 3
    console(f"fill {sx + 3} {y} {sz} {sx + 6} {y + 4} {sz} minecraft:obsidian")
    console(f"fill {sx + 4} {y + 1} {sz} {sx + 5} {y + 3} {sz} minecraft:air")
    console(f"setblock {sx + 4} {y + 1} {sz} minecraft:fire", 1.0)
    console(f"tp {name} {sx + 5.0} {y + 1} {sz + 0.5}")
    deadline = time.time() + 30
    while time.time() < deadline and "to minecraft:the_nether" not in e2e.server_log():
        time.sleep(0.5)
    time.sleep(12)
    nether = e2e.WORK / "dimensions-view-nether.png"
    e2e.screenshot(pid, nether)
    print("screenshot:", nether)
    # Look around from the exit portal's front.
    m = re.findall(r"to minecraft:the_nether at \[(-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\]", e2e.server_log())
    if m:
        x, yy, z = (float(v) for v in m[-1])
        console(f"execute in minecraft:the_nether run tp {name} {x} {yy} {z + 3.5} 180 10", 6)
        nether2 = e2e.WORK / "dimensions-view-nether-2.png"
        e2e.screenshot(pid, nether2)
        print("screenshot:", nether2)

    # An end portal block in the Nether leads to the End's platform.
    console(f"execute in minecraft:the_nether run tp {name} 0.5 200 0.5", 2)
    console("execute in minecraft:the_nether run setblock 0 199 0 minecraft:obsidian")
    console("execute in minecraft:the_nether run setblock 3 200 0 minecraft:end_portal")
    time.sleep(3)
    console(f"execute in minecraft:the_nether run tp {name} 3.5 200 0.5")
    deadline = time.time() + 30
    while time.time() < deadline and "to minecraft:the_end" not in e2e.server_log():
        time.sleep(0.5)
    time.sleep(15)
    end = e2e.WORK / "dimensions-view-end.png"
    e2e.screenshot(pid, end)
    print("screenshot:", end)
    console(f"execute in minecraft:the_end run tp {name} 30 90 0 90 20", 12)
    end2 = e2e.WORK / "dimensions-view-end-2.png"
    e2e.screenshot(pid, end2)
    print("screenshot:", end2)

    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN|went from|portal|gateway|generation", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
