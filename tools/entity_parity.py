"""Differential tests of kiln-entity against vanilla 26.3.

1. Runs tools/EntityVectors.java: a vanilla dedicated server started in-process (in
   <work>/wp4-entities/server, port 25592) that ticks entity scenarios by hand and records
   every entity's state after every tick, into <work>/wp4-entities/vectors.jsonl.
2. Runs `cargo test -p kiln-entity --test parity` with KILN_PARITY=1, which replays each
   scenario in Rust and compares every recorded value bit for bit.

usage: python tools/entity_parity.py [--filter NAME] [--skip-java] [--out FILE]
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
    ap.add_argument("--out", default=str(WORK / "wp4-entities" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = WORK / "wp4-entities" / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "EntityVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... EntityVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "EntityVectors" in l or "Exception" in l or "Error" in l or "\tat " in l]
        print("\n".join(lines[-40:]))
        if p.returncode != 0:
            sys.exit(f"EntityVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_PARITY="1", KILN_ENTITY_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-entity", "--test", "parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
