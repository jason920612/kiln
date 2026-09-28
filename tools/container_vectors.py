"""Differential tests of Kiln's container block entities (hoppers, furnaces, comparators)
against vanilla 26.3.

1. Runs tools/ContainerVectors.java: a vanilla dedicated server started in-process (in a
   `server` directory next to the output, port 25597) where each scenario's blocks are placed
   and whole level ticks run; the watched containers (items, hopper cooldowns, furnace timers),
   block states and comparator outputs after every tick go to
   <work>/wp15-containers/vectors.jsonl.
2. Runs `cargo test -p kiln-sim container_parity` with KILN_CONTAINER_VECTORS set:
   container_parity replays each scenario through the simulation and compares every tick
   exactly.

usage: python tools/container_vectors.py [--filter NAME] [--skip-java] [--out FILE]
Set KILN_DATAPACK to the vanilla datapack (work/generated) when running from a worktree.
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
    ap.add_argument("--out", default=str(WORK / "wp15-containers" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "ContainerVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... ContainerVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "ContainerVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"ContainerVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_CONTAINER_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "container_parity", "--", "--nocapture",
                              "--test-threads=1"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
