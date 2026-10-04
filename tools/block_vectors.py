"""Differential tests of what blocks do on their own (random ticks, scheduled ticks, what block
changes set off) against vanilla 26.3.

1. Runs tools/BlockTickVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port $KILN_HARNESS_PORT or 25581) that records the scenarios to
   <work>/wp44/block_vectors.jsonl.
2. Runs `cargo test -p kiln-blocks --lib block_parity` with KILN_BLOCK_VECTORS set, which replays
   each scenario through kiln-blocks on a test level and compares every op.

usage: python tools/block_vectors.py [--filter REGEX] [--skip-java] [--out FILE]
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
    ap.add_argument("--filter", help="only scenarios whose name matches this regex (Java) / contains one of a|b (Rust)")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "wp44" / "block_vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "BlockTickVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... BlockTickVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "BlockTickVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"BlockTickVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_BLOCK_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "--release", "-p", "kiln-blocks", "--lib", "block_parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
