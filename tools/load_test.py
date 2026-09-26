"""Load test: start a release Kiln server, run kiln-bot against it, and report the server's
tick statistics while the bots are connected.

usage: python tools/load_test.py [--count 1000] [--groups 20] [--behavior crowd] [--duration 90]
                                 [--port 25570] [--view-distance 2] [--spacing 48]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
EXT = ".exe" if os.name == "nt" else ""
SERVER = ROOT / "target" / "release" / f"kiln{EXT}"
BOT = ROOT / "target" / "release" / f"kiln-bot{EXT}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--groups", type=int, default=20)
    ap.add_argument("--spacing", type=float, default=48.0)
    ap.add_argument("--behavior", default="crowd")
    ap.add_argument("--radius", type=float)
    ap.add_argument("--duration", type=int, default=90)
    ap.add_argument("--rate", type=float, default=100.0)
    ap.add_argument("--port", type=int, default=25570)
    ap.add_argument("--view-distance", type=int, default=2)
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()

    if not a.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "kiln-server", "-p", "kiln-bot"], cwd=ROOT, check=True)
    exe = WORK / f"kiln-load-{a.port}{EXT}"
    shutil.copy2(SERVER, exe)
    env = dict(os.environ, KILN_PORT=str(a.port), KILN_MAX_PLAYERS=str(a.count + 10), KILN_OPS="LoadBot*",
               RUST_LOG="info")
    server = subprocess.Popen([str(exe)], cwd=WORK, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    lines = []
    threading.Thread(target=lambda: [lines.append(l.rstrip()) for l in server.stdout], daemon=True).start()
    try:
        time.sleep(1.5)
        cmd = [str(BOT), "--addr", f"127.0.0.1:{a.port}", "--count", str(a.count), "--rate", str(a.rate),
               "--behavior", a.behavior, "--duration", str(a.duration), "--name-prefix", "LoadBot",
               "--groups", str(a.groups), "--group-spacing", str(a.spacing), "--teleport-to-group",
               "--view-distance", str(a.view_distance), "--report-interval", "10"]
        if a.radius is not None:
            cmd += ["--radius", str(a.radius)]
        bots = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
        print("--- bots ---")
        print(bots.stdout[-3000:])
        # Tick reports logged while everyone was connected (the second half of the run).
        reports = [l for l in lines if "players," in l and "ms" in l]
        full = [l for l in reports if re.search(rf"\b{a.count} players", l)]
        print("--- server tick reports (all bots online) ---")
        print("\n".join((full or reports)[-8:]))
    finally:
        try:
            server.stdin.write("stop\n")
            server.stdin.flush()
            server.wait(timeout=30)
        except Exception:
            server.kill()


if __name__ == "__main__":
    sys.exit(main())
