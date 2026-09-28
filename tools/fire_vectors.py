"""Differential tests of Kiln's fire (spread, burning, aging, faces) against vanilla 26.3.

1. Runs tools/FireVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port 25593) that records the scenarios to
   <work>/wp-fire/vectors.jsonl.
2. Runs `cargo test -p kiln-blocks --lib fire_parity` with KILN_FIRE_VECTORS set, which
   replays each scenario through kiln-blocks' fire on a test level and compares every round.

usage: python tools/fire_vectors.py [--skip-java] [--out FILE]
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
    ap.add_argument("--out", default=str(WORK / "wp-fire" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "FireVectors.java"), str(out)]
        print("$ java ... FireVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "FireVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"FireVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_FIRE_VECTORS=str(out))
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-blocks", "--lib", "fire_parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
