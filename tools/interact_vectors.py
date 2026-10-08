"""Differential tests of Kiln's player interactions (equipping armor by right click, editing, dyeing
and waxing signs, editing books, picking items) against vanilla 26.3.

1. Runs tools/InteractVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port $KILN_HARNESS_PORT or the first free one of 25581-25583)
   where a mock player is set up and serverbound packets run through the packet listener; after
   every step the inventory, the clientbound packets, the watched blocks and the item entities
   go to <work>/wp44/interact/vectors.jsonl.
2. Runs `cargo test -p kiln-sim interact_parity` with KILN_INTERACT_VECTORS set: the simulation
   replays each scenario and must end every step in the same state.

usage: python tools/interact_vectors.py [--filter NAME] [--skip-java] [--out FILE] [--profile dev|release]
"""

import argparse
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


def free_port():
    """$KILN_HARNESS_PORT, else the first free port of 25581-25583 (waits while all are busy)."""
    if os.environ.get("KILN_HARNESS_PORT"):
        return os.environ["KILN_HARNESS_PORT"]
    for _ in range(600):
        for port in (25581, 25582, 25583):
            with socket.socket() as s:
                try:
                    s.bind(("0.0.0.0", port))
                except OSError:
                    continue
            return str(port)
        time.sleep(2)
    sys.exit("no free harness port")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", help="only scenarios whose name contains this (or matches it as a regex)")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--skip-rust", action="store_true", help="only record")
    ap.add_argument("--out", default=str(WORK / "wp44" / "interact" / "vectors.jsonl"))
    ap.add_argument("--profile", default="release")
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "InteractVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... InteractVectors.java", out, flush=True)
        # Another worker may take the port between the check and the server's bind: try again.
        for attempt in range(4):
            env = dict(os.environ, KILN_HARNESS_PORT=free_port())
            p = subprocess.run(cmd, cwd=server_dir, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
            lines = [l for l in (p.stdout + p.stderr).splitlines()
                     if "InteractVectors" in l or "DEBUG" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
            print("\n".join(lines[-60:]))
            if p.returncode == 0 or "server did not start" not in (p.stdout + p.stderr):
                break
            time.sleep(5)
        if p.returncode != 0:
            sys.exit(f"InteractVectors failed ({p.returncode})")
    if args.skip_rust:
        return
    env = dict(os.environ, KILN_INTERACT_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    cargo = ["cargo", "test"] + (["--release"] if args.profile == "release" else []) + [
        "-p", "kiln-sim", "--lib", "interact_parity", "--", "--nocapture"]
    sys.exit(subprocess.call(cargo, cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
