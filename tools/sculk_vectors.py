"""Differential tests of Kiln's game events, vibrations and sculk sensors against vanilla 26.3.

1. Runs tools/SculkVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port $KILN_MOB_PORT, default 25614) where each scenario's
   blocks are placed and whole level ticks run with commands before chosen ticks; the watched
   block states and sculk sensors' vibration state after every tick go to
   <work>/m6s3-warden/sculk_vectors.jsonl.
2. Runs `cargo test -p kiln-sim --lib sculk_parity` with KILN_SCULK_VECTORS set: sculk_parity
   replays each scenario through the simulation and compares every tick exactly.

usage: python tools/sculk_vectors.py [--filter NAME] [--skip-java] [--out FILE]
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", help="only scenarios whose name contains this")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "m6s3-warden" / "sculk_vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "sculk-server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "SculkVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... SculkVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "SculkVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"SculkVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_SCULK_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "sculk_parity", "--", "--nocapture",
                              "--test-threads=1"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
