"""wp73: the idle tick with thousands of containers (wp70: scanning every container for dispensers each region tick
was ~10% of it). Starts Kiln (servers73.sh), forces 4 chunks, fills a 20x20x20 cube with chests (8000 block
entities), waits, then reads `/tick query` several times (P50 of the last 100 ticks = the idle tick).

On the VM:  python3 chest73.py [--bin PATH] [--queries N]
"""

import argparse
import subprocess
import sys
import time

from fl73 import Server, TOOLS


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin")
    ap.add_argument("--queries", type=int, default=8)
    ap.add_argument("--block", default="chest")
    args = ap.parse_args()
    subprocess.run([str(TOOLS / "servers73.sh"), "K", "start"] + ([args.bin] if args.bin else []), check=True)
    s = Server("K")
    t0 = time.time()
    while True:
        try:
            s.query(timeout=5)
            break
        except (TimeoutError, FileNotFoundError):
            if time.time() - t0 > 600:
                sys.exit("server did not start")
    try:
        for c in ["gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false", "gamerule random_tick_speed 0",
                  "forceload add 0 0 31 31", f"fill 0 100 0 19 119 19 {args.block}", "wp73marker"]:
            s.send(c)
        s.wait_for("wp73marker")
        time.sleep(10)
        p50s = []
        for _ in range(args.queries):
            time.sleep(5.5)
            p50, _ = s.query()
            p50s.append(p50)
        empty = []
        s.send(f"fill 0 100 0 19 119 19 air")
        time.sleep(8)
        for _ in range(max(2, args.queries // 2)):
            time.sleep(5.5)
            p50, _ = s.query()
            empty.append(p50)
        mean = lambda v: sum(v) / len(v)
        print(f"idle tick P50 with 8000 {args.block}s: mean {mean(p50s):.3f} ms {p50s}; without: mean {mean(empty):.3f} ms {empty}", flush=True)
    finally:
        subprocess.run([str(TOOLS / "servers73.sh"), "K", "stop"])


if __name__ == "__main__":
    main()
