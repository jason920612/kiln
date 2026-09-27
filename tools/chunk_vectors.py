"""Run tools/ChunkVectors.java against the server jar and its libraries.

Writes vanilla chunk generation dumps (biomes, fill, surface, carvers; Mojang-derived data:
never commit) to <work>/wp3-worldgen/chunks unless another directory is given.

usage: python tools/chunk_vectors.py [out dir] [--regions N] [--bench N] [seed...]
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
    out = WORK / "wp3-worldgen" / "chunks"
    if args and not args[0].startswith("-") and not args[0].lstrip("-").isdigit() and args[0] != "random":
        out = Path(args.pop(0))
    cmd = ["java", "-Xmx8g", "-cp", classpath(), str(ROOT / "tools" / "ChunkVectors.java"), str(out), *args]
    sys.exit(subprocess.call(cmd, cwd=WORK))
