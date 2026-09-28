"""Differential tests of Kiln's mobs against vanilla 26.3.

1. Runs tools/MobVectors.java: a vanilla dedicated server started in-process (in
   <work>/m6-mobs/server, port 25597) that ticks mob scenarios by hand with pinned seeds and
   records every mob's state after every tick into <work>/m6-mobs/vectors.jsonl.
2. Runs `cargo test -p kiln-entity --test mob_parity` with KILN_MOB_VECTORS set, which replays
   each scenario in Rust and compares position, velocity, rotations, health, target, running
   goals and the ticks the player was hit.

usage: python tools/mob_vectors.py [--filter NAME] [--skip-java] [--out FILE]
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
    ap.add_argument("--out", default=str(WORK / "m6-mobs" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "MobVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... MobVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "MobVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"MobVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_MOB_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-entity", "--test", "mob_parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
