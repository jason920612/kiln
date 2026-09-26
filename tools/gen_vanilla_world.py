"""Generate a reference world with the vanilla 26.3 server (fixed seed, a force-loaded area
around spawn), for Anvil loading and parity tests.

usage: python tools/gen_vanilla_world.py [--seed N] [--radius CHUNKS] [--out DIR]
"""

import argparse
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", default="12345")
    ap.add_argument("--radius", type=int, default=12, help="force-load (2r)x(2r) chunks around 0,0")
    ap.add_argument("--out", default=str(WORK / "vanilla-world"))
    ap.add_argument("--port", type=int, default=25591)
    a = ap.parse_args()

    out = Path(a.out)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    (out / "eula.txt").write_text("eula=true\n")
    (out / "server.properties").write_text(
        "\n".join(
            [
                f"level-seed={a.seed}",
                f"server-port={a.port}",
                "online-mode=false",
                "enable-rcon=false",
                "spawn-protection=0",
                "max-tick-time=-1",
                "sync-chunk-writes=false",
            ]
        )
        + "\n"
    )

    p = subprocess.Popen(
        ["java", "-Xmx3G", "-jar", str(WORK / "server.jar"), "--nogui"],
        cwd=out,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    lines = []

    def reader():
        for line in p.stdout:
            lines.append(line.rstrip())
            print(line.rstrip(), flush=True)

    threading.Thread(target=reader, daemon=True).start()

    def wait_for(text, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if any(text in l for l in lines):
                return True
            if p.poll() is not None:
                sys.exit("vanilla server exited")
            time.sleep(0.5)
        return False

    def cmd(c):
        p.stdin.write(c + "\n")
        p.stdin.flush()

    if not wait_for("Done (", 300):
        p.kill()
        sys.exit("server did not start")
    r = a.radius
    # /forceload handles at most 256 chunks per call.
    step = 16
    for x0 in range(-r, r, step):
        for z0 in range(-r, r, step):
            x1, z1 = min(x0 + step, r) - 1, min(z0 + step, r) - 1
            cmd(f"forceload add {x0 * 16} {z0 * 16} {x1 * 16 + 15} {z1 * 16 + 15}")
    # Let generation finish, then save and stop.
    time.sleep(max(20, r * r * 4 // 60))
    cmd("save-all flush")
    wait_for("Saved the game", 300)
    cmd("stop")
    p.wait(timeout=120)
    print("world at", out / "world")


if __name__ == "__main__":
    main()
