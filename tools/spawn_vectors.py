"""Differential test of natural spawning's structure overrides against vanilla 26.3.

1. Runs tools/SpawnVectors.java: a vanilla dedicated server started in-process (in
   <out dir>/server, a generated world with structures) that generates the chunks around
   fortresses, bastions, swamp huts, monuments, outposts, trial chambers, ancient cities... and records,
   for sample positions in and around every structure, the list `NaturalSpawner.mobsAt` returns per
   mob category, with the chunks' `structures` data, into <work>/wp44/spawner/structure_spawns.jsonl.
2. Runs `cargo test -p kiln-sim structure_spawns` with KILN_STRUCTURE_SPAWN_VECTORS set, which asks
   Kiln's spawn table the same questions and compares the lists.

usage: python tools/spawn_vectors.py [--skip-java] [--seed N] [--out FILE]
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
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--seed", default="1234")
    ap.add_argument("--out", default=str(WORK / "wp44" / "spawner" / "structure_spawns.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "structure-server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "--add-opens", "java.base/java.util=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "SpawnVectors.java"), str(out), args.seed]
        print("$ java ... SpawnVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "SpawnVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"SpawnVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_STRUCTURE_SPAWN_VECTORS=str(out))
    sys.exit(subprocess.call(["cargo", "test", "--release", "-p", "kiln-sim", "--lib", "structure_spawns", "--", "--nocapture"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
