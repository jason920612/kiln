"""wp73 forceload test: `/forceload add` right after start on Kiln and vanilla 26.3 (same seed, fresh noise world),
each command's tick read with `/tick query` (P99 of the last 100 ticks = the slowest of them), the wall time
until the command's end marker, the feedback, and whether a block in the far corner of each area can be set
in the same tick (the chunks are loaded when the command returns).

On the VM:  python3 fl73.py K|V [--bin PATH] [--out FILE]
Servers come from servers73.sh (Kiln 25590, vanilla 25597).
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

HOME = Path.home()
W = HOME / "wp73"
TOOLS = Path(__file__).resolve().parent
P99 = re.compile(r"P50: ([\d.]+) ?ms P95: ([\d.]+) ?ms P99: ([\d.]+) ?ms")
FEEDBACK = re.compile(r"(?:\]: |kiln_sim::commands: )(.*)$")

# (name, forceload command, a block in its far corner set right after it in the same tick)
CASES = [
    # wp70's start-up set (157 chunks)
    ("wp70 a 42", "forceload add 0 0 111 95", "setblock 111 300 95 stone"),
    ("wp70 b 6", "forceload add 96 96 143 127", "setblock 143 300 127 stone"),
    ("wp70 c 9", "forceload add 192 192 239 239", "setblock 239 300 239 stone"),
    ("wp70 d 100", "forceload add 288 288 447 447", "setblock 447 300 447 stone"),
    # the per-command limit, twice, away from everything else
    ("256 e", "forceload add 2000 2000 2255 2255", "setblock 2255 300 2255 stone"),
    ("256 f", "forceload add -3008 1008 -2753 1263", "setblock -2753 300 1263 stone"),
    # over the limit: refused
    ("272 g", "forceload add 5008 5008 5279 5263", None),
    # loaded already: nothing changes
    ("again e", "forceload add 2000 2000 2255 2255", None),
    # let them go, wait until they are unloaded and saved, then force them again: read from storage
    ("remove e", "forceload remove 2000 2000 2255 2255", None),
    ("reload e", "forceload add 2000 2000 2255 2255", "setblock 2255 301 2255 stone"),
]
# seconds to wait after a case (default 6: the heavy tick leaves the 100-tick window)
WAIT_AFTER = {"remove e": 40}


class Server:
    def __init__(self, which):
        self.dir = W / which
        self.log = self.dir / "server.log"
        self.fifo = open(self.dir / "console", "w", buffering=1)
        self.pos = 0

    def lines(self):
        with open(self.log, encoding="utf-8", errors="replace") as f:
            f.seek(self.pos)
            text = f.read()
            self.pos = f.tell()
        return text.splitlines()

    def send(self, cmd):
        self.fifo.write(cmd + "\n")
        self.fifo.flush()

    def query(self, timeout=600.0):
        self.send("tick query")
        end = time.time() + timeout
        while time.time() < end:
            for l in self.lines():
                m = P99.search(l)
                if m:
                    return float(m.group(1)), float(m.group(3))
            time.sleep(0.05)
        raise TimeoutError("no tick query answer")

    def wait_for(self, text, timeout=900.0):
        end = time.time() + timeout
        got = []
        while time.time() < end:
            for l in self.lines():
                if text in l:
                    return got
                got.append(l)
            time.sleep(0.02)
        raise TimeoutError(text)


def feedback(lines):
    out = []
    for l in lines:
        m = FEEDBACK.search(l)
        if m and "Unknown or incomplete" not in m.group(1) and "<--[HERE]" not in m.group(1):
            out.append(m.group(1))
    return out


def run(args):
    which = args.which
    subprocess.run([str(TOOLS / "servers73.sh"), which, "start"] + ([args.bin] if args.bin else []), check=True)
    s = Server(which)
    t0 = time.time()
    while True:
        try:
            s.query(timeout=5)
            break
        except (TimeoutError, FileNotFoundError):
            if time.time() - t0 > 600:
                sys.exit("server did not start")
    print(f"[{which}] up after {time.time() - t0:.0f}s", flush=True)
    results = []
    try:
        for i, (name, cmd, probe) in enumerate(CASES):
            s.query()
            t = time.time()
            s.send(cmd)
            if probe:
                s.send(probe)
            s.send(f"wp73marker{i}")
            lines = s.wait_for(f"wp73marker{i}")
            wall = time.time() - t
            p50, p99 = s.query()
            fb = feedback(lines)
            r = {"case": name, "cmd": cmd, "wall_s": round(wall, 2), "worst_ms": p99, "idle_ms": p50, "feedback": fb}
            results.append(r)
            print(f"[{which}] {name:10s} worst tick {p99:9.1f} ms  wall {wall:6.2f} s  | " + " | ".join(fb)[:300], flush=True)
            time.sleep(WAIT_AFTER.get(name, 6))
        if args.out:
            Path(args.out).write_text(json.dumps(results, indent=1))
    finally:
        subprocess.run([str(TOOLS / "servers73.sh"), which, "stop"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("which", choices=["K", "V"])
    ap.add_argument("--bin")
    ap.add_argument("--out")
    run(ap.parse_args())


if __name__ == "__main__":
    main()
