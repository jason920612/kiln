"""Run tools/ItemVectors.java against the unobfuscated server jar and its libraries.

usage: python tools/item_vectors.py defaults <out.json>
       python tools/item_vectors.py corpus <out-dir> [vector files...]
       python tools/item_vectors.py corpus            (writes <work>/wp2-items with tools/item_vectors/*.txt)
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
    if args == ["corpus"]:
        args = ["corpus", str(WORK / "wp2-items")] + [str(p) for p in sorted((ROOT / "tools" / "item_vectors").glob("*.txt"))]
    cmd = ["java", "-Xss8m", "-cp", classpath(), str(ROOT / "tools" / "ItemVectors.java")] + args
    sys.exit(subprocess.call(cmd, cwd=WORK))
