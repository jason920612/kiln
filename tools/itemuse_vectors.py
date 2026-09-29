"""Differential tests of Kiln's player item uses against vanilla 26.3: the view ray buckets
aim with (Level.clip with outline shapes and source fluids), Projectile.shootFromRotation
(bows, tridents, thrown items) and the crossbow's shot vector.

1. Runs tools/ItemUseVectors.java: a vanilla dedicated server started in-process (in a
   `server` directory next to the output, port 25594) that records the scenarios to
   <work>/wp-itemuse/vectors.jsonl.
2. Runs `cargo test -p kiln-sim --lib item_parity` with KILN_ITEMUSE_VECTORS set, which
   replays each scenario through Kiln and prints the agreement.

usage: python tools/itemuse_vectors.py [--skip-java] [--out FILE]
(from a worktree: set KILN_WORK to the repository's work directory)
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
    ap.add_argument("--out", default=str(WORK / "wp-itemuse" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "ItemUseVectors.java"), str(out)]
        print("$ java ... ItemUseVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "ItemUseVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            print("\n".join((p.stdout + p.stderr).splitlines()[-80:]))
            sys.exit(f"ItemUseVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_ITEMUSE_VECTORS=str(out))
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "item_parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
