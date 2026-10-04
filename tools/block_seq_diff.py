"""Finds the first random tick whose draws differ between vanilla and Kiln (wp44 block vectors).

Runs the Java harness and the Rust replay for one scenario with KILN_SEQ_TRACE set: both write, before
every random tick of an `rt` op, "SEQ x,y,z <level random state> <block>" (<prefix>_vanilla.txt and
<prefix>_kiln.txt). The first line where the states differ is the tick before which the two sides
drew a different number of random numbers (blocks the replay never reached after its first
difference are not compared).

usage: python tools/block_seq_diff.py <scenario-name> [--skip-java]
"""

import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK  # noqa: E402


def main():
    name = sys.argv[1]
    out_dir = WORK / "wp44" / "seq"
    out_dir.mkdir(parents=True, exist_ok=True)
    prefix = str(out_dir / name)
    for suffix in ("_vanilla.txt", "_kiln.txt"):
        Path(prefix + suffix).unlink(missing_ok=True)
    vectors = out_dir / f"{name}.jsonl"
    env = dict(os.environ, KILN_SEQ_TRACE=prefix)
    cmd = [sys.executable, str(ROOT / "tools" / "block_vectors.py"), "--filter", name, "--out", str(vectors)]
    if "--skip-java" in sys.argv:
        cmd.append("--skip-java")
    subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True)
    vanilla = Path(prefix + "_vanilla.txt").read_text().splitlines()
    kiln = Path(prefix + "_kiln.txt").read_text().splitlines()
    print(f"{len(vanilla)} ticks traced in vanilla, {len(kiln)} in Kiln")
    for i, (v, k) in enumerate(zip(vanilla, kiln)):
        if v != k:
            print(f"first difference at tick {i} (before it the draws differ); around it:")
            for j in range(max(0, i - 3), min(len(vanilla), len(kiln), i + 3)):
                print(" vanilla", vanilla[j])
                print(" kiln   ", kiln[j])
            return
    print("random states equal over", min(len(vanilla), len(kiln)), "ticks")


if __name__ == "__main__":
    main()
