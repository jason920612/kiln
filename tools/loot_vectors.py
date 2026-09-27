"""Run tools/LootVectors.java against the server jar and its libraries.

usage: python tools/loot_vectors.py [<out-dir> [<contexts per table> [<seed> [<synthetic tables .json>]]]]
       (defaults: <work>/wp4-loot, 8 contexts per table, seed 1)
"""

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


if __name__ == "__main__":
    args = sys.argv[1:]
    out = args[0] if len(args) > 0 else str(WORK / "wp4-loot")
    contexts = args[1] if len(args) > 1 else "8"
    seed = args[2] if len(args) > 2 else "1"
    datapack = str(WORK / "generated")
    cmd = ["java", "-Xss8m", "-cp", classpath(), str(ROOT / "tools" / "LootVectors.java"), out, contexts, seed, datapack]
    if len(args) > 3:
        # Hand-written tables (crates/kiln-loot/tests/synthetic.json) instead of the datapack's.
        cmd.append(str(Path(args[3]).resolve()))
    sys.exit(subprocess.call(cmd, cwd=WORK))
