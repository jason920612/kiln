"""Start a release server with extra settings, join it with the real 26.3 client, wait for the
world to load and take a screenshot of the game window (work/client-view.png).

usage: python tools/client_view.py [--port 25570] [--wait 20] [--keep] [KEY=VALUE ...]
e.g.   python tools/client_view.py KILN_GENERATOR=noise KILN_SEED=12345
"""

import argparse
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25570)
    ap.add_argument("--wait", type=int, default=20, help="seconds to let chunks load after joining")
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("settings", nargs="*")
    a = ap.parse_args()
    for kv in a.settings:
        k, v = kv.split("=", 1)
        os.environ[k] = v
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    e2e.start_server(a.port)
    name = "KilnView"
    # The client JVM sometimes dies during startup; try a few times like tools/e2e.py.
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
        if e2e.alive(pid):
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")
    time.sleep(a.wait)
    shot = e2e.WORK / "client-view.png"
    e2e.screenshot(pid, shot)
    print("screenshot:", shot)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
