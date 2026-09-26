"""Run tools/WorldgenVectors.java against the server jar and its libraries.

Writes vanilla density function / noise parity vectors (Mojang-derived data: never commit)
to <work>/wp2-worldgen unless another directory is given.

usage: python tools/worldgen_vectors.py [out dir] [seed...]
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
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else WORK / "wp2-worldgen"
    seeds = sys.argv[2:]
    cmd = ["java", "-Xmx4g", "-cp", classpath(), str(ROOT / "tools" / "WorldgenVectors.java"), str(out), *seeds]
    sys.exit(subprocess.call(cmd, cwd=WORK))
