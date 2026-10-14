"""Differential test of Kiln's horse, donkey, mule and llama breeding against vanilla 26.3 (wp63).

1. Runs tools/EquineBreedVectors.java: a vanilla dedicated server started in-process breeds thousands of pairs of
   tame parents (varied health, speed, jump strength, llama strength and coat, horse coat and markings) with a
   pinned random and records the parents, the child and the parent's next random draw into
   <work>/equine-breed/vectors.jsonl.
2. Runs `cargo test -p kiln-entity --lib equine_vectors` with KILN_EQUINE_VECTORS set: every child must have exactly
   the vanilla attributes, strength and coat, and the random must have been drawn the same number of times.

usage: python tools/equine_breed.py [--samples N] [--skip-java] [--out FILE]
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
    ap.add_argument("--samples", type=int, default=400, help="breedings per parent pair (8 pairs)")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "equine-breed" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "--add-opens", "java.base/java.util=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "EquineBreedVectors.java"), str(out), str(args.samples)]
        print("$ java ... EquineBreedVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines() if "EquineBreedVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"EquineBreedVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_EQUINE_VECTORS=str(out))
    sys.exit(subprocess.call(["cargo", "test", "--release", "-p", "kiln-entity", "--lib", "equine_vectors", "--", "--nocapture"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
