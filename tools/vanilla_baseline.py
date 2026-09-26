"""Run the same bot workload against the vanilla 26.3 server and report its tick times
(`/tick query`), as a baseline for Kiln's numbers.

usage: python tools/vanilla_baseline.py [--count 300] [--behavior crowd] [--duration 70]
"""

import argparse
import shutil
import subprocess
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"
BOT = ROOT / "target" / "release" / "kiln-bot.exe"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--behavior", default="crowd")
    ap.add_argument("--duration", type=int, default=70)
    ap.add_argument("--port", type=int, default=25593)
    ap.add_argument("--view-distance", type=int, default=10)
    a = ap.parse_args()

    base = WORK / "vanilla-baseline"
    if base.exists():
        shutil.rmtree(base)
    base.mkdir(parents=True)
    (base / "eula.txt").write_text("eula=true\n")
    (base / "server.properties").write_text(
        "\n".join(
            [
                f"server-port={a.port}",
                "online-mode=false",
                "level-type=minecraft\\:flat",
                "difficulty=peaceful",
                "spawn-protection=0",
                "max-players=2000",
                f"view-distance={a.view_distance}",
                f"simulation-distance={a.view_distance}",
                "enforce-secure-profile=false",
                "white-list=false",
                "network-compression-threshold=256",
                "max-tick-time=-1",
            ]
        )
        + "\n"
    )
    p = subprocess.Popen(
        ["java", "-Xmx4G", "-jar", str(WORK / "server.jar"), "--nogui"],
        cwd=base, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
    )
    lines = []
    threading.Thread(target=lambda: [lines.append(l.rstrip()) for l in p.stdout], daemon=True).start()

    def wait_for(text, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if any(text in l for l in lines):
                return True
            time.sleep(0.2)
        return False

    def cmd(c):
        p.stdin.write(c + "\n")
        p.stdin.flush()

    try:
        if not wait_for("Done (", 300):
            raise SystemExit("vanilla did not start")
        # Let the spawn area settle before the bots arrive.
        time.sleep(5)
        bots = subprocess.Popen(
            [str(BOT), "--addr", f"127.0.0.1:{a.port}", "--count", str(a.count), "--rate", "100",
             "--behavior", a.behavior, "--duration", str(a.duration), "--report-interval", str(a.duration)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        )
        # Measure over the second half of the run, when everyone has joined.
        time.sleep(a.count / 100 + a.duration / 2)
        cmd("tick query")
        time.sleep(a.duration / 2 - 5)
        n = len(lines)
        cmd("tick query")
        time.sleep(2)
        report = [l for l in lines[n:] if "tick" in l.lower() or "P50" in l or "P95" in l or "P99" in l or "ms" in l]
        out, _ = bots.communicate(timeout=120)
        print("--- bots ---")
        print("\n".join(l for l in out.splitlines() if "joined" in l or "received" in l or "teleports" in l))
        print("--- vanilla /tick query ---")
        print("\n".join(report))
    finally:
        cmd("stop")
        try:
            p.wait(timeout=60)
        except subprocess.TimeoutExpired:
            p.kill()


if __name__ == "__main__":
    main()
